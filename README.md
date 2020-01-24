# cgc 
cgc is a compacting garbage collector


## How it works?
### Mark-Sweep-Compact ( major collection ):
This phase is the most important part in the GC, if heap is fragmented Mark-Compact may compact the heap so allocation is possible again. But as you understand we can't just always compact heap, for this there are `FreeList::fragmentation()` function, if return value is `>= 0.50` then GC enables compaction phase.

- Marking 

    In this phase GC identifies live objects and mark them.
- Sweeping

    All objects that not marked in previous phase now "freed" and finalizers is invoked.
- Compaction

    This phase computes forwarding pointers and then shifts objects to start of the heap.
    This phase will not happen at every collection. Compaction occurs only when the heap is fragmented by more than fifty percent.
### Incremental Mark&Sweep
Incremental phase is perfomed only at `alloc` call and if heap size is bigger than threshold, but incremental collection may also occur when there are no memory left or allocation fails. To make this algorithm work correctly there are `cgc::write_barrier` function. The write barrier is a piece of code that runs just before a pointer store occurs and records just enough information to make sure that live objects don't get collected.

- Marking
    Mark all objects from `scan_list`
- Sweeping
    All "white" objects sweeped, black objects is alive, gray objects is now objects that will be scanned in the next collection.

    
## Allocation
cgc uses simple bump and freelist allocation.
When there are some memory for bump allocation then bump allocation will happen until bump pointer will reach heap end. When bump pointer is equal to heap end freelist starts it's work, if it fails then collection occurs and collection may compact heap so bump allocation will be possible again.

## Thread-safety and blocking threads
Current implementation doesn't block thread until thread tries to access GC pointer when collection happen in another thread, this means when collection happens other threads may do some other work that doesn't associated with GC. cgc itself does not provide any sync primitives, and you can't get "safe" access to one object from one thread from another thread.

