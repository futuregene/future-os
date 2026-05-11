package agent

import "sync"

// PendingMessageQueue is a thread-safe buffered message queue with a drain mode.
// Mode "all" drains all pending messages at once. Mode "one-at-a-time" drains
// exactly one message per drain call, leaving any remainder for the next drain.
type PendingMessageQueue struct {
	mu   sync.Mutex
	ch   chan string
	Mode string // "all" or "one-at-a-time"
}

// NewPendingMessageQueue creates a new message queue with the given capacity and mode.
// capacity sets the internal channel buffer size.
// mode controls drain behavior: "all" drains everything, "one-at-a-time" drains a single message.
func NewPendingMessageQueue(capacity int, mode string) *PendingMessageQueue {
	if mode == "" {
		mode = "all"
	}
	return &PendingMessageQueue{
		ch:   make(chan string, capacity),
		Mode: mode,
	}
}

// Enqueue adds a message to the queue. If the queue is full, the message is dropped.
func (q *PendingMessageQueue) Enqueue(msg string) {
	select {
	case q.ch <- msg:
	default:
		// queue full, drop
	}
}

// Drain returns all (or one, depending on Mode) pending messages.
func (q *PendingMessageQueue) Drain() []string {
	q.mu.Lock()
	defer q.mu.Unlock()

	var msgs []string
	for {
		select {
		case msg := <-q.ch:
			msgs = append(msgs, msg)
			if q.Mode == "one-at-a-time" {
				return msgs
			}
		default:
			return msgs
		}
	}
}

// Len returns the number of pending messages in the queue.
func (q *PendingMessageQueue) Len() int {
	return len(q.ch)
}

// Clear drains and discards all pending messages.
func (q *PendingMessageQueue) Clear() {
	for {
		select {
		case <-q.ch:
		default:
			return
		}
	}
}
