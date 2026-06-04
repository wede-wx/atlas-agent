type Listener = (payload: any) => void;

const listeners: Record<string, Listener[]> = {};
const lastListenerErrors: Record<string, number> = {};

export const EventBus = {
  subscribe(event: string, cb: Listener) {
    if (!listeners[event]) listeners[event] = [];
    if (!listeners[event].includes(cb)) listeners[event].push(cb);
    return () => this.unsubscribe(event, cb);
  },
  unsubscribe(event: string, cb: Listener) {
    if (!listeners[event]) return;
    listeners[event] = listeners[event].filter(fn => fn !== cb);
  },
  dispatch(event: string, payload?: any) {
    if (!listeners[event]) return;
    listeners[event].forEach(fn => {
      try {
        fn(payload);
      } catch (error) {
        const now = Date.now();
        if (now - (lastListenerErrors[event] || 0) > 2000) {
          lastListenerErrors[event] = now;
          console.warn(`[Aura EventBus] listener failed for ${event}`, error);
        }
      }
    });
  },
};
