/** 选中时加粗的标签，始终按粗体文字的宽度占位（`.steady-label`）。 */
export function SteadyLabel({ text }: { text: string }) {
  return <span className="steady-label" data-label={text}>{text}</span>;
}

/** 可选的芯片：按下后标签变粗，但芯片不变宽，后面的芯片不会被挤动。 */
export function Chip({ label, pressed, onClick }: { label: string; pressed: boolean; onClick(): void }) {
  return <button type="button" className="chip" aria-pressed={pressed} onClick={onClick}><SteadyLabel text={label} /></button>;
}
