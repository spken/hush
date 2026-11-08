use std::{
    any::Any,
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    fmt, mem,
    rc::{Rc, Weak},
    time::Duration,
};

use slint::{Model, ModelNotify, ModelTracker, Timer, TimerMode};

/// Rows kept loaded; a `ListView` shows far fewer at once.
const CACHED_ROWS: usize = 300;
/// Rows loaded together around one that is missing.
const PAGE: usize = 100;
/// While rows keep going missing, they are asked for at most this often.
const COALESCE: Duration = Duration::from_millis(100);

/// A list for a `ListView`, held as entity keys. Row data is loaded on demand, through the
/// `load` callback and then `fill`, and kept only for the most recently loaded rows.
pub struct KeyListModel<T> {
    this: Weak<Self>,
    keys: RefCell<Vec<u32>>,
    rows: RefCell<Rows<T>>,
    /// Bumped by `set_keys`: rows loaded before it stay on show but are loaded again.
    generation: Cell<u32>,
    /// Keys asked for and not filled yet.
    requested: RefCell<HashSet<u32>>,
    /// Keys to ask for at the next flush.
    wanted: RefCell<Vec<u32>>,
    flush: Timer,
    load: Box<dyn Fn(Vec<u32>)>,
    notify: ModelNotify,
}

struct Rows<T> {
    by_key: HashMap<u32, (T, u32)>,
    /// Loaded keys, oldest first.
    order: VecDeque<u32>,
}

impl<T: Clone + Default + 'static> KeyListModel<T> {
    /// An empty list that calls `load` with the keys of rows it needs.
    pub fn new(load: impl Fn(Vec<u32>) + 'static) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            this: this.clone(),
            keys: RefCell::default(),
            rows: RefCell::new(Rows {
                by_key: HashMap::new(),
                order: VecDeque::new(),
            }),
            generation: Cell::new(0),
            requested: RefCell::default(),
            wanted: RefCell::default(),
            flush: Timer::default(),
            load: Box::new(load),
            notify: ModelNotify::default(),
        })
    }

    /// Replaces the list. Rows already loaded stay on show until fresh ones arrive.
    pub fn set_keys(&self, keys: Vec<u32>) {
        *self.keys.borrow_mut() = keys;
        self.generation.set(self.generation.get().wrapping_add(1));
        self.requested.borrow_mut().clear();
        self.notify.reset();
    }

    /// Stores the loaded rows this list asked for and redraws them where they are on show.
    /// Rows other lists asked for are ignored.
    pub fn fill(&self, loaded: impl IntoIterator<Item = (u32, T)>) {
        let generation = self.generation.get();
        let mut filled = HashSet::new();
        {
            let mut rows = self.rows.borrow_mut();
            let mut requested = self.requested.borrow_mut();
            for (key, row) in loaded {
                if !requested.remove(&key) {
                    continue;
                }
                if rows.by_key.insert(key, (row, generation)).is_none() {
                    rows.order.push_back(key);
                }
                filled.insert(key);
            }
            while rows.order.len() > CACHED_ROWS {
                if let Some(oldest) = rows.order.pop_front() {
                    rows.by_key.remove(&oldest);
                }
            }
        }
        if filled.is_empty() {
            return;
        }
        let changed: Vec<usize> = (self.keys.borrow().iter().enumerate())
            .filter(|(_, key)| filled.contains(key))
            .map(|(row, _)| row)
            .collect();
        for row in changed {
            self.notify.row_changed(row);
        }
    }

    /// Asks for the page around `row`: at once when nothing was asked for recently, else
    /// with the next flush.
    fn request(&self, row: usize) {
        {
            let keys = self.keys.borrow();
            let rows = self.rows.borrow();
            let mut requested = self.requested.borrow_mut();
            let mut wanted = self.wanted.borrow_mut();
            for &key in keys.iter().skip(row / PAGE * PAGE).take(PAGE) {
                let fresh = rows
                    .by_key
                    .get(&key)
                    .is_some_and(|(_, generation)| *generation == self.generation.get());
                if !fresh && requested.insert(key) {
                    wanted.push(key);
                }
            }
        }
        if !self.flush.running() {
            self.send();
            let this = self.this.clone();
            self.flush.start(TimerMode::Repeated, COALESCE, move || {
                if let Some(this) = this.upgrade()
                    && !this.send()
                {
                    this.flush.stop();
                }
            });
        }
    }

    /// Calls `load` with the wanted keys; `false` when there were none.
    fn send(&self) -> bool {
        let wanted = mem::take(&mut *self.wanted.borrow_mut());
        if wanted.is_empty() {
            return false;
        }
        (self.load)(wanted);
        true
    }
}

impl<T: Clone + Default + 'static> Model for KeyListModel<T> {
    type Data = T;

    fn row_count(&self) -> usize {
        self.keys.borrow().len()
    }

    fn row_data(&self, row: usize) -> Option<T> {
        let key = *self.keys.borrow().get(row)?;
        let cached = (self.rows.borrow().by_key.get(&key))
            .map(|(data, generation)| (data.clone(), *generation == self.generation.get()));
        if !cached.as_ref().is_some_and(|(_, fresh)| *fresh)
            && !self.requested.borrow().contains(&key)
        {
            self.request(row);
        }
        Some(cached.map(|(data, _)| data).unwrap_or_default())
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl<T> fmt::Debug for KeyListModel<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyListModel")
            .field("rows", &self.keys.borrow().len())
            .finish_non_exhaustive()
    }
}
