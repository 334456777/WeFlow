/** A label set bold when it is chosen, which takes the width of its bold text all the time (`.steady-label`). */
export function SteadyLabel({ text }: { text: string }) {
  return <span className="steady-label" data-label={text}>{text}</span>;
}

/** A choice chip: pressed, its label turns bold without making the chip wider, so the chips after it stay put. */
export function Chip({ label, pressed, onClick }: { label: string; pressed: boolean; onClick(): void }) {
  return <button type="button" className="chip" aria-pressed={pressed} onClick={onClick}><SteadyLabel text={label} /></button>;
}
