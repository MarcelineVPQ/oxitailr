// The words of the sections' status lines ("Name on", "Name off",
// "Name: value"), one place for every page (as in the DS5 configurator).
export const onOff = (name: string, on: boolean): string => `${name} ${on ? "on" : "off"}`;
export const named = (name: string, value: string | number): string => `${name}: ${value}`;
