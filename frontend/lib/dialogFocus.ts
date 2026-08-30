export function wrappedDialogFocusIndex(activeIndex: number, focusableCount: number, shiftKey: boolean): number | null {
  if (focusableCount <= 0) return null;
  if (activeIndex < 0) return shiftKey ? focusableCount - 1 : 0;
  if (shiftKey && activeIndex === 0) return focusableCount - 1;
  if (!shiftKey && activeIndex === focusableCount - 1) return 0;
  return null;
}
