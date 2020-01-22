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
        println!("trace foo {}", self.x);
        match &mut self.next {
            Some(x) => f(x),
            _ => (),
        }
    }
}

impl Finalizer for Foo {
    fn finalize(&mut self) {
        println!("finalize foo {}", self.x);
    }
}

fn main() {
    let mut gc = GlobalCollector::new(1024 * 1024 * 2); // 256 kb heap
    {
        let _v = gc.alloc(vec![12, 3]);
        let _y = gc.alloc(Foo { x: 4, next: None });
    }
    let x = gc.alloc(Foo { x: 3, next: None });
    gc.collect();
    let z = gc.alloc(Foo {
        x: 5,
        next: Some(Heap::from(&x)),
    });

    println!("{}", x.get().x);

    gc.collect();
    println!("{}", z.get().x);
}
