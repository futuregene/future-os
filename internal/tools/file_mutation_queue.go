package tools

import (
	"sync"
)

// FileMutationQueue serializes mutations to the same file path, ensuring that
// multiple goroutines do not concurrently read-modify-write the same file.
//
// Usage:
//
//	q := NewFileMutationQueue()
//	q.Enqueue("/path/to/file", func() {
//	    // safe to read and write /path/to/file here
//	})
type FileMutationQueue struct {
	mu     sync.Mutex
	queues map[string]chan func()
}

// NewFileMutationQueue creates a new queue. Workers are started lazily per
// file path — a dedicated goroutine processes mutations for each path.
func NewFileMutationQueue() *FileMutationQueue {
	return &FileMutationQueue{
		queues: make(map[string]chan func()),
	}
}

// Enqueue schedules fn to run exclusively for filePath. It blocks until fn
// completes, providing natural back-pressure. Calls for different file paths
// proceed concurrently.
func (q *FileMutationQueue) Enqueue(filePath string, fn func()) {
	q.mu.Lock()
	ch, exists := q.queues[filePath]
	if !exists {
		ch = make(chan func(), 1)
		q.queues[filePath] = ch
		go q.worker(ch)
	}
	q.mu.Unlock()

	done := make(chan struct{})
	ch <- func() {
		fn()
		close(done)
	}
	<-done
}

// worker processes mutation functions sequentially for a single file path.
func (q *FileMutationQueue) worker(ch chan func()) {
	for fn := range ch {
		fn()
	}
}

// Close shuts down all workers. Pending mutations complete before the
// channels are closed. After Close, further Enqueue calls will panic.
func (q *FileMutationQueue) Close() {
	q.mu.Lock()
	defer q.mu.Unlock()

	for path, ch := range q.queues {
		close(ch)
		delete(q.queues, path)
	}
}
