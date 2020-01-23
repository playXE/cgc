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
    {
        let x = gc.alloc(Foo { x: 3, next: None });
    }
    let n = gc.alloc(Foo { x: 4, next: None }).to_heap();
    let free = gc.alloc(Foo {
        x: 42,
        next: Some(n),
    });
    gc.force_compact();
}
