# Rooting Guide

This guide explains the basics of using `cgc` in your programs. 
Since `cgc` is a moving GC it's very important that it knows about each and every pointer to a GC thing in a system.

## What is a GC thing pointer?
"GC thing"  is the term used to refer to memory allocated and managed by cgc.
So you can think that "GC Thing" is a thing that implements `Traceable` trait and rooted or stored in a rooted object.

## GC thing on the stack

- `Rooted<T>`:
    All GC thing pointers stored on stack (i.e local variables and function parameters) must use the `cgc::rooting::Rooted<T>`.
    This is RAII structure returned from `gc.alloc` (`gc_alloc` if you use static collector) that gets destroyed at end of the scope. You could also use `Arc` or `Rc` with `Rooted<T>`.

    Example:
    ```rust
    let mut gc = GlobalCollector::new(1024 * 4); // 4kb heap
    { /* scope start */
        let foo: Rooted<i32> = gc.alloc(42);
        // do something with foo...
    } /* scope end */
    gc.collect();
    ```
    As you can see we allocate `i32` on GC heap and then we can do somethign with it. When we done with `foo` it will reach scope end or you can just use `drop(foo)` but you actually shouldn't use `drop(foo)` if you already use GC. After scope end we also invoke `gc.collect()` (`gc_collect` if you use static collector), this function will trigger garbage collection cycle and will sweep `foo` ( "free" it)

## GC things on the heap
- `Heap<T>`:

    GC thing pointers on the heap must be wrapped in `Heap<T>`. `Heap<T>` **pointers must also continue to be traced in the normal way**, which is covered below.
    
    `Heap<T>` doesn't require invoking `gc.alloc`(`gc_alloc`), and can be constructed from `Rooted<T>` using `Heap::from` or `Rooted::<T>::to_heap`

    There are `Heap::get` and `Heap::get_mut` that user could use to get access to value. It's UB to access gc'ed value.

# Tracing
- 
    All GC pointers stored on the heap must be traced or they will be freed. Almost always GC pointers is traced through rooted objects that located on the stack.
