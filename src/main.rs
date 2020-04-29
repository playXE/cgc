extern crate cgc_single_threaded as cgc;

use cgc::api::*;

// Simple linked list.
#[derive(Debug)]
struct Foo(Option<Handle<Foo>>);

impl Traceable for Foo {
    fn trace_with(&self, tracer: &mut Tracer) {
        self.0.trace_with(tracer);
    }
}

impl Finalizer for Foo {
    fn finalize(&mut self) {
        println!("GCed");
    }
}

fn main() {
    let mut heap = cgc::heap::Heap::new(1024, 2048); // 1kb new space,2kb old space.
    let value = heap.allocate(Foo(None));
    let value2 = heap.allocate(Foo(Some(value.to_heap())));
    println!("{:?}", value2);
}
