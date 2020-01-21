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
        //println!("trace foo {}", self.x);
        match &mut self.next {
            Some(x) => f(x),
            _ => (),
        }
    }
}

impl Finalizer for Foo {
    fn finalize(&mut self) {
        //println!("finalize foo {}", self.x);
        self.x += 1;
    }
}

fn main() {
    let n = time::Instant::now();
    let mut gc = GlobalCollector::new(1024 * 1024 * 100);
    {
        let mut v = gc.alloc(vec![]);

        for _ in 0..1000000 {
            v.get_mut()
                .push(gc.alloc(Foo { x: 0, next: None }).to_heap());
        }
        println!("done");
    }

    gc.compact();
    println!("{}", n.elapsed().whole_milliseconds());
}
