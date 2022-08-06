// Copyright 2017 The xi-editor Authors.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Monitoring files and directories.
//!
//! This module contains `FileWatcher` and related types, responsible for
//! monitoring changes to files and directories. Under the hood it is a
//! thin wrapper around some concrete type provided by the
//! [`notify`](https://docs.rs/notify) crate; the implementation is
//! platform dependent, and may be using kqueue, fsevent, or another
//! low-level monitoring system.
//!
//! Our wrapper provides a few useful features:
//!
//! - All `watch` calls are associated with a `WatchToken`; this
//! allows for the same path to be watched multiple times,
//! presumably by multiple interested parties. events are delivered
//! once-per token.
//!
//! - There is the option (via `FileWatcher::watch_filtered`) to include
//! a predicate along with a path, to filter paths before delivery.
//!
//! - We are integrated with the xi_rpc runloop; events are queued as
//! they arrive, and an idle task is scheduled.

use notify::{event::*, recommended_watcher, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use zi::ComponentLink;

use crate::editor::buffer::{BufferMessage, BuffersMessage};
use crate::editor::{BufferId, Editor};

/// Wrapper around a `notify::Watcher`. It runs the inner watcher
/// in a separate thread, and communicates with it via a [crossbeam channel].
/// [crossbeam channel]: https://docs.rs/crossbeam-channel
pub struct FileWatcher {
    inner: RecommendedWatcher,
    state: Arc<Mutex<WatcherState>>,
}

#[derive(Debug, Default)]
struct WatcherState {
    events: EventQueue,
    watchees: Vec<Watchee>,
}

/// Tracks a registered 'that-which-is-watched'.
#[doc(hidden)]
struct Watchee {
    path: PathBuf,
    recursive: bool,
    token: WatchToken,
    filter: Option<Box<PathFilter>>,
}

/// Token provided to `FileWatcher`, to associate events with the corresponding buffer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchToken(pub BufferId);

pub type EventQueue = VecDeque<(WatchToken, Event)>;

pub type PathFilter = dyn Fn(&Path) -> bool + Send + 'static;

impl FileWatcher {
    /// Create a new `FileWatcher` using an existing `ComponentLink<Editor>`. This link will be
    /// used to notify the editor of changes to watched files.
    pub fn new(link: ComponentLink<Editor>) -> Self {
        let state = Arc::new(Mutex::new(WatcherState::default()));
        let state_clone = state.clone();

        let event_fn = move |res: notify::Result<notify::Event>| match res {
            Ok(event) => {
                let mut state = state_clone.lock().unwrap();
                let WatcherState {
                    ref mut events,
                    ref mut watchees,
                } = *state;

                watchees
                    .iter()
                    .filter(|w| w.wants_event(&event))
                    .map(|w| w.token)
                    .for_each(|t| events.push_back((t, event.clone())));

                for (token, _) in events {
                    let buffer_message = BufferMessage::Refresh;
                    link.send(BuffersMessage::new(token.0, buffer_message).into());
                }
            }
            Err(e) => log::error!("Error creating FileWatcher. {}", e),
        };

        let inner = recommended_watcher(event_fn).unwrap();

        FileWatcher { inner, state }
    }

    /// Begin watching `path`. As `Event`s (documented in the [notify](https://docs.rs/notify)
    /// crate) arrive, they are stored with the associated `token`.
    pub fn watch(&mut self, path: &Path, recursive: bool, token: WatchToken) {
        self.watch_impl(path, recursive, token, None);
    }

    /// Like `watch`, but taking a predicate function that filters delivery of events based on
    /// their path.
    #[allow(dead_code)]
    pub fn watch_filtered<F>(&mut self, path: &Path, recursive: bool, token: WatchToken, filter: F)
    where
        F: Fn(&Path) -> bool + Send + 'static,
    {
        let filter = Box::new(filter) as Box<PathFilter>;
        self.watch_impl(path, recursive, token, Some(filter));
    }

    fn watch_impl(
        &mut self,
        path: &Path,
        recursive: bool,
        token: WatchToken,
        filter: Option<Box<PathFilter>>,
    ) {
        let path = match path.canonicalize() {
            Ok(ref p) => p.to_owned(),
            Err(e) => {
                log::warn!("error watching {:?}: {:?}", path, e);
                return;
            }
        };

        let mut state = self.state.lock().unwrap();

        let w = Watchee {
            path,
            recursive,
            token,
            filter,
        };
        let mode = mode_from_bool(w.recursive);

        if !state.watchees.iter().any(|w2| w.path == w2.path) {
            if let Err(e) = self.inner.watch(&w.path, mode) {
                log::warn!("watching error {:?}", e);
            }
        }

        state.watchees.push(w);
    }

    /// Removes the provided token/path pair from the watch list. Does not stop watching this path,
    /// if it is associated with other tokens.
    pub fn unwatch(&mut self, path: &Path, token: WatchToken) {
        let mut state = self.state.lock().unwrap();

        let idx = state
            .watchees
            .iter()
            .position(|w| w.token == token && w.path == path);

        if let Some(idx) = idx {
            let removed = state.watchees.remove(idx);
            if !state.watchees.iter().any(|w| w.path == removed.path) {
                if let Err(e) = self.inner.unwatch(&removed.path) {
                    log::warn!("unwatching error {:?}", e);
                }
            }

            // if this was recursive, check if any child paths need to be manually re-added
            if removed.recursive {
                // do this in two steps because we've borrowed mutably up top
                let to_add = state
                    .watchees
                    .iter()
                    .filter(|w| w.path.starts_with(&removed.path))
                    .map(|w| (w.path.to_owned(), mode_from_bool(w.recursive)))
                    .collect::<Vec<_>>();

                for (path, mode) in to_add {
                    if let Err(e) = self.inner.watch(&path, mode) {
                        log::warn!("watching error {:?}", e);
                    }
                }
            }
        }
    }
}

impl Watchee {
    fn wants_event(&self, event: &Event) -> bool {
        match &event.kind {
            EventKind::Create(CreateKind::Any)
            | EventKind::Remove(RemoveKind::Any)
            | EventKind::Modify(ModifyKind::Any)
            | EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)) => {
                if event.paths.len() == 1 {
                    self.applies_to_path(&event.paths[0])
                } else {
                    log::info!(
                        "Rejecting event {:?} with incorrect paths. Expected 1 found {}.",
                        event,
                        event.paths.len()
                    );
                    false
                }
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                if event.paths.len() == 2 {
                    //There will be two paths. First is "from" and other is "to".
                    self.applies_to_path(&event.paths[0]) || self.applies_to_path(&event.paths[1])
                } else {
                    log::info!(
                        "Rejecting event {:?} with incorrect paths. Expected 2 found {}.",
                        event,
                        event.paths.len()
                    );
                    false
                }
            }
            _ => false,
        }
    }

    fn applies_to_path(&self, path: &Path) -> bool {
        let general_case = if path.starts_with(&self.path) {
            (self.recursive || self.path == path) || path.parent() == Some(&self.path)
        } else {
            false
        };

        if let Some(ref filter) = self.filter {
            general_case && filter(path)
        } else {
            general_case
        }
    }
}

impl fmt::Debug for Watchee {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Watchee path: {:?}, r {}, t {} f {}",
            self.path,
            self.recursive,
            self.token.0,
            self.filter.is_some()
        )
    }
}

fn mode_from_bool(is_recursive: bool) -> RecursiveMode {
    if is_recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    }
}
