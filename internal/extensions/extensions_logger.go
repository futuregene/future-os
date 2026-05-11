package extensions

// ---------------------------------------------------------------------------
// Logger — simple leveled logger interface for extensions
// ---------------------------------------------------------------------------

// Logger is the logging interface available to extensions via ExtensionContext.
type Logger interface {
	Info(format string, args ...interface{})
	Warn(format string, args ...interface{})
	Error(format string, args ...interface{})
	Debug(format string, args ...interface{})
}

// StdLogger is a basic Logger implementation that writes to a Printf-style function.
type StdLogger struct {
	Infof  func(format string, args ...interface{})
	Warnf  func(format string, args ...interface{})
	Errorf func(format string, args ...interface{})
	Debugf func(format string, args ...interface{})
}

func (l *StdLogger) Info(format string, args ...interface{})  { l.Infof(format, args...) }
func (l *StdLogger) Warn(format string, args ...interface{})  { l.Warnf(format, args...) }
func (l *StdLogger) Error(format string, args ...interface{}) { l.Errorf(format, args...) }
func (l *StdLogger) Debug(format string, args ...interface{}) { l.Debugf(format, args...) }

// ---------------------------------------------------------------------------
// Event — lightweight event type for the extension event bus
// ---------------------------------------------------------------------------

// Event is a named event with an arbitrary payload.
type Event struct {
	Name string
	Data interface{}
}

// EventBus provides a simple publish/subscribe mechanism for extensions
// to communicate with each other and with the host application.
type EventBus struct {
	subscribers map[string][]chan Event
}

// NewEventBus creates a new EventBus.
func NewEventBus() *EventBus {
	return &EventBus{
		subscribers: make(map[string][]chan Event),
	}
}

// Subscribe registers a channel to receive events with the given name.
// The channel should be buffered to avoid blocking the publisher.
func (eb *EventBus) Subscribe(eventName string, ch chan Event) {
	eb.subscribers[eventName] = append(eb.subscribers[eventName], ch)
}

// Unsubscribe removes a channel from the given event name.
func (eb *EventBus) Unsubscribe(eventName string, ch chan Event) {
	subs := eb.subscribers[eventName]
	for i, s := range subs {
		if s == ch {
			eb.subscribers[eventName] = append(subs[:i], subs[i+1:]...)
			return
		}
	}
}

// Publish sends an event to all subscribers. Non-blocking: if a subscriber's
// buffer is full, the event is dropped for that subscriber.
func (eb *EventBus) Publish(ev Event) {
	for _, ch := range eb.subscribers[ev.Name] {
		select {
		case ch <- ev:
		default:
			// drop if subscriber is not ready
		}
	}
}
