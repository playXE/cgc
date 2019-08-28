extern crate gc_rs;

use gc_rs::*;

fn main() {
    let mut gc = GarbageCollector::new(None);
    gc.verbose = true;
    let v: GC<Vec<GC<String>>> = gc.allocate(vec![]);

    gc.add_root(v.clone());
    gc.collect();
    v.borrow_mut().push(gc.allocate("Hello!".to_owned()));
    v.borrow_mut().push(gc.allocate("Hello!".to_owned()));
    v.borrow_mut().push(gc.allocate("Hello!".to_owned()));
    v.borrow_mut().push(gc.allocate("Hello!".to_owned()));
    gc.collect();
    v.borrow_mut().clear();
    gc.collect();
}
