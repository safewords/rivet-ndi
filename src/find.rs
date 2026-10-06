//! Source discovery (`NDIlib_find_*`).

use std::ffi::CString;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::ffi::{self, Lib};
use crate::{Error, Result, c_string, cstr_to_string};

/// An NDI source: its name (`MACHINE (Stream)`) and, when discovery found
/// it, the address the runtime reached it at.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Source {
    pub name: String,
    pub url: Option<String>,
}

impl Source {
    /// A source by name alone: the runtime resolves it when a receiver
    /// connects, without a finder.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: None,
        }
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// How discovery looks for sources.
#[derive(Debug, Clone)]
pub struct FindOptions {
    /// List sources running on this machine too (default `true`).
    pub show_local_sources: bool,
    /// The groups to look in, comma-separated; `None` is the runtime's
    /// default (`public`, unless the NDI access manager says otherwise).
    pub groups: Option<String>,
    /// Further machines to ask directly, comma-separated IPs, for networks
    /// where mDNS does not reach.
    pub extra_ips: Option<String>,
}

impl Default for FindOptions {
    fn default() -> Self {
        Self {
            show_local_sources: true,
            groups: None,
            extra_ips: None,
        }
    }
}

/// A running discovery. Sources accumulate in the background from the
/// moment it is made until it is dropped.
pub struct Finder {
    lib: Arc<Lib>,
    instance: ffi::Instance,
}

// SAFETY: a finder instance may be used from any one thread at a time.
unsafe impl Send for Finder {}

impl Finder {
    pub(crate) fn new(lib: Arc<Lib>, options: &FindOptions) -> Result<Self> {
        let groups = options
            .groups
            .as_deref()
            .map(|g| c_string(g, "the NDI groups"))
            .transpose()?;
        let extra = options
            .extra_ips
            .as_deref()
            .map(|g| c_string(g, "the extra NDI addresses"))
            .transpose()?;
        let create = ffi::FindCreate {
            show_local_sources: options.show_local_sources,
            p_groups: groups.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
            p_extra_ips: extra.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
        };
        // SAFETY: the strings outlive the call, which copies them.
        let instance = unsafe { (lib.find_create_v2)(&create) };
        if instance.is_null() {
            return Err(Error::CreateFailed("NDIlib_find_create_v2"));
        }
        Ok(Self { lib, instance })
    }

    /// Block until the source list changes or `timeout` passes; whether it
    /// changed.
    pub fn wait(&self, timeout: Duration) -> bool {
        // SAFETY: a live instance.
        unsafe { (self.lib.find_wait_for_sources)(self.instance, millis(timeout)) }
    }

    /// The sources known now.
    pub fn current(&self) -> Vec<Source> {
        let mut n = 0u32;
        // SAFETY: the array is valid until the next call on this finder or
        // its destruction; it is copied out before either.
        unsafe {
            let p = (self.lib.find_get_current_sources)(self.instance, &mut n);
            if p.is_null() {
                return Vec::new();
            }
            std::slice::from_raw_parts(p, n as usize)
                .iter()
                .filter_map(|s| {
                    Some(Source {
                        name: cstr_to_string(s.p_ndi_name)?,
                        url: cstr_to_string(s.p_url_address).filter(|u| !u.is_empty()),
                    })
                })
                .collect()
        }
    }

    /// Every source seen within `wait`, sorted by name.
    pub fn sources_within(&self, wait: Duration) -> Result<Vec<Source>> {
        let deadline = Instant::now() + wait;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            if left.is_zero() || !self.wait(left) {
                break;
            }
        }
        let mut all = self.current();
        all.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(all)
    }

    /// The one source `query` names, as soon as it appears (see
    /// [`crate::Ndi::find_source`]).
    pub fn find(&self, query: &str, wait: Duration) -> Result<Source> {
        let deadline = Instant::now() + wait;
        loop {
            let seen = self.current();
            if let Some(found) = pick(query, &seen)? {
                return Ok(found);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Error::SourceNotFound {
                    query: query.to_string(),
                    waited_ms: wait.as_millis(),
                    seen: if seen.is_empty() {
                        "none".into()
                    } else {
                        names(&seen)
                    },
                });
            }
            self.wait(left.min(Duration::from_millis(250)));
        }
    }
}

impl Drop for Finder {
    fn drop(&mut self) {
        // SAFETY: created by `find_create_v2`, destroyed once.
        unsafe { (self.lib.find_destroy)(self.instance) }
    }
}

/// The source `query` names among `seen`: the exact name, ignoring case;
/// else the one name containing it. Several containing it is an error, not
/// a guess.
pub(crate) fn pick(query: &str, seen: &[Source]) -> Result<Option<Source>> {
    let q = query.to_lowercase();
    if let Some(s) = seen.iter().find(|s| s.name.to_lowercase() == q) {
        return Ok(Some(s.clone()));
    }
    let partial: Vec<&Source> = seen
        .iter()
        .filter(|s| s.name.to_lowercase().contains(&q))
        .collect();
    match partial.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some((*one).clone())),
        many => Err(Error::AmbiguousSource {
            query: query.to_string(),
            matches: many
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}

fn names(seen: &[Source]) -> String {
    seen.iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn millis(d: Duration) -> u32 {
    d.as_millis().min(u128::from(u32::MAX)) as u32
}

/// Keeps a `CString` and the `Source` struct pointing into it together.
pub(crate) struct RawSource {
    _name: CString,
    _url: Option<CString>,
    pub raw: ffi::Source,
}

impl RawSource {
    pub(crate) fn new(source: &Source) -> Result<Self> {
        let name = c_string(&source.name, "the NDI source name")?;
        let url = source
            .url
            .as_deref()
            .map(|u| c_string(u, "the NDI source address"))
            .transpose()?;
        let raw = ffi::Source {
            p_ndi_name: name.as_ptr(),
            p_url_address: url.as_ref().map_or(std::ptr::null(), |u| u.as_ptr()),
        };
        Ok(Self {
            _name: name,
            _url: url,
            raw,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen() -> Vec<Source> {
        [
            "STUDIO (Camera 1)",
            "STUDIO (Camera 2)",
            "EDIT-PC (Program)",
        ]
        .into_iter()
        .map(Source::named)
        .collect()
    }

    #[test]
    fn an_exact_name_wins_ignoring_case() {
        let got = pick("studio (camera 1)", &seen()).unwrap().unwrap();
        assert_eq!(got.name, "STUDIO (Camera 1)");
    }

    #[test]
    fn a_unique_part_of_a_name_finds_it() {
        let got = pick("program", &seen()).unwrap().unwrap();
        assert_eq!(got.name, "EDIT-PC (Program)");
        assert!(pick("Camera 3", &seen()).unwrap().is_none());
    }

    #[test]
    fn a_part_two_names_share_is_refused_by_name() {
        let err = pick("camera", &seen()).unwrap_err().to_string();
        assert!(
            err.contains("STUDIO (Camera 1), STUDIO (Camera 2)"),
            "{err}"
        );
    }
}
