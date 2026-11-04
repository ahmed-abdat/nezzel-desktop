import { defineStore } from 'pinia';
import { ref } from 'vue';

let idCounter = 0;

export type ToastStyle = 'success' | 'info' | 'error' | 'warning' | 'subtle';

export type Toast = {
  id: number;
  message: string;
  style: ToastStyle;
  duration: number;
  /** One inline action (e.g. Undo). Running it closes the toast. */
  action?: { label: string; run: () => void };
};

export const useToastStore = defineStore('toast', () => {
  const toasts = ref<Toast[]>([]);

  const showToast = (message: string, options: Partial<Omit<Toast, 'id' | 'message'>> = {}) => {
    const id = ++idCounter;
    const toast: Toast = {
      id,
      message,
      style: options.style ?? 'info',
      duration: options.duration ?? 3000,
      action: options.action,
    };

    toasts.value.push(toast);

    if (toast.duration > 0) {
      setTimeout(() => removeToast(id), toast.duration);
    }
    return id;
  };

  const removeToast = (id: number) => {
    const index = toasts.value.findIndex(t => t.id === id);
    if (index >= 0) toasts.value.splice(index, 1);
  };

  return {
    toasts,
    showToast,
    removeToast,
  };
});
