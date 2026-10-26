// The account setup (#216) as a dialog over the workspace, which plays its exit before it leaves the top layer.
import { useEffect, useRef } from 'react';
import type { Profile } from '../api/setup';
import type { JobsState } from '../jobs/useJobs';
import { usePresence } from '../motion';
import { SetupWizard } from './SetupWizard';

export function SetupDialog({ open, profile, jobs, onChanged, onClose, onDone }: {
  open: boolean; profile: Profile | undefined; jobs: JobsState; onChanged(): void; onClose(): void; onDone(): void;
}) {
  const shown = usePresence(open);
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    if (shown && dialog && !dialog.open) dialog.showModal?.();
    else if (!shown && dialog?.open) dialog.close();
  }, [shown]);
  return <dialog className="setup-dialog" ref={ref} aria-label="账号配置" data-state={shown?.closing ? 'closing' : 'open'}
    onCancel={event => { event.preventDefault(); onClose(); }}>
    {shown && <SetupWizard profile={profile} jobs={jobs} onChanged={onChanged} onClose={onClose} onDone={onDone} />}
  </dialog>;
}
