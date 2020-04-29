# cgc 
cgc is generational copying garbage collector.

This branch implements signle-threaded version that does not have anything to deal with concurrency and synchronization.

## Algorithm

cgc uses semispace garbage collection to keep GC simple and generations to improve GC latency.

## Who this crate is for
- For people developing programming language runtime, no matter is language dynamically or statically typed.
- Game developers where lot's of assets is loaded and some of them not cleared automatically.
- For someone who is doing big & complex graph structures and do not want to mess with reference counting.
- For people that deal with huge heaps and fragmentation, cgc removes fragmentation using copying garbage collection.

## Logging support
cgc supports logging. To alloc printing trace add feature `trace-gc` to features and to pring GC timings
add `trace-gc-timings` feature.

## Comparison to other GC crates
### [broom](https://github.com/zesterer/broom)
Advantages of `broom`:
- Less dependencies (depends only on one crate `hashbrown`).
- Very small & simple implementation.


Disadvantages of `broom`:
- Mark'n Sweep algorithm. Mark&sweep may be really slow on huge heaps.
- No memory defragmentation and generations. Without memory defragmentation it is really slow to allocate memory when heap is fragmented
and without generations collections may be really slow since GC will collect *entire* heap.
- No concurrent garbage collection support.

### [gc](https://github.com/Manishearth/rust-gc)
Advantages of `gc`: 
- No dependencies
- Easy to make GC object,just use `#[derive(Trace)]`.


Disadvantages of `gc`: 


`gc` crate has the same disadvantages as `broom` crate, it uses mark'n sweep and does not have any memory defragmentation.
## Usage

cgc is simple to use all you have to do is implement `Traceable` and `Finalizer` for your type and you now have GC object!
```rust
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
    {
        let value = heap.allocate(Foo(None));
        let value2 = heap.allocate(Foo(Some(value.to_heap())));
        println!("{:?}", value2);
    }
    heap.collect(); // value and value2 is GCed.
    
}
```