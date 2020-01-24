extern crate cgc;

use cgc::collector::*;
use cgc::rooting::*;
use cgc::trace::*;

struct Foo {
    x: i32,
    next: Option<Heap<Foo>>,
}
impl Traceable for Foo {
    fn trace_with<'a>(&'a mut self, mut f: impl FnMut(&'a mut dyn HeapTrait)) {
        match &mut self.next {
            Some(x) => f(x),
            _ => (),
        }
    }
}

impl Finalizer for Foo {
    fn finalize(&mut self) {}
}

fn main() {
    simple_logger::init().unwrap();
    let mut gc = GlobalCollector::new(1024 * 1024);
    let mut x: Rooted<Vec<cgc::Heap<i32>>> = gc.alloc(vec![]);
    let y = gc.alloc(42);

    cgc::write_barrier(&x.to_heap(), &y.to_heap());
    x.get_mut().push(y.to_heap());
}
