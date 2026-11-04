export { default as Input } from './Input.vue';

/** What a template ref on <Input> exposes (the SFC instance type is opaque to typescript-eslint). */
export interface InputApi {
  focus: () => void;
  select: () => void;
}
