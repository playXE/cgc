# cgc 
cgc is a compacting garbage collector


## How it works?
cgc includes 3 phases:
- Marking 

    In this phase GC identifies live objects and mark them.
- Sweeping

    All objects that not marked in previous phase now "freed" and finalizers is invoked.
- Compaction

    This phase computes forwarding pointers and then shifts objects to start of the heap.
    This phase will not happen at every collection. Compaction occurs only when the heap is fragmented by more than fifty percent.
    
    

## Allocation
cgc uses simple bump and freelist allocation.
When there are some memory for bump allocation then bump allocation will happen until bump pointer will reach heap end. When bump pointer is equal to heap end freelist starts it's work, if it fails then collection occurs and collection may compact heap so bump allocation will be possible again.

## Thread-safety and blocking threads
Current implementation doesn't block thread until thread tries to access GC pointer when collection happen in another thread, this means when collection happens other threads may do some other work that doesn't associated with GC. cgc itself does not provide any sync primitives, and you can't get "safe" access to one object from one thread from another thread.

## Parallel/incremental/concurrent GC
cgc is stop-the-world but version with parallel marking and incremental marking is planned.

## Generations
cgc is not generational and doesn't have "young" and "old" space. This means GC will always collect entire heap, but generations may be added to incremental version of GC.

