use crate::*;

use std::collections::*;

impl<T: Trace> Trace for VecDeque<T> {
    fn trace(&self) {
        self.iter().for_each(|x| x.trace());
    }
}

impl<T: Trace> Trace for LinkedList<T> {
    fn trace(&self) {
        self.iter().for_each(|x| x.trace());
    }
}

impl<T: Trace> Trace for HashSet<T> {
    fn trace(&self) {
        self.iter().for_each(|x| x.trace());
    }
}

impl<K: Trace + Hash + Eq, T: Trace> Trace for HashMap<K, T> {
    fn trace(&self) {
        self.iter().for_each(|(key, val)| {
            key.trace();
            val.trace()
        });
    }
}

impl<T: Trace> Trace for BTreeSet<T> {
    fn trace(&self) {
        self.iter().for_each(|x| x.trace());
    }
}

impl<K: Trace, V: Trace> Trace for BTreeMap<K, V> {
    fn trace(&self) {
        self.iter().for_each(|(key, val)| {
            key.trace();
            val.trace()
        });
    }
}

use hashlink::*;

impl<K: Trace + Hash + Eq, T: Trace> Trace for LinkedHashMap<K, T> {
    fn trace(&self) {
        self.iter().for_each(|(key, val)| {
            key.trace();
            val.trace()
        });
    }
}

impl<T: Trace> Trace for LinkedHashSet<T> {
    fn trace(&self) {
        self.iter().for_each(|x| x.trace());
    }
}
