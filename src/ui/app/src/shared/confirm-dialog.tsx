import type { ReactNode } from 'react';
import { Button, Dialog } from '@heartwood/ui';
import type { IconComponent } from '@heartwood/ui';

export interface ConfirmDialogProps {
  title: string;
  /** What the commit does, as a verb phrase on the button: "Stop sharing". */
  commit: string;
  /** The commit removes or interrupts something: the destructive button, and Cancel is the focus. */
  danger?: boolean;
  icon?: IconComponent;
  isDisabled?: boolean;
  isPending?: boolean;
  /** Called when the commit is pressed. The dialog closes unless `keepOpen`. */
  onConfirm: () => void;
  /** Keeps the dialog open after the commit, for a request whose answer the dialog shows. */
  keepOpen?: boolean;
  /** Called however the dialog closes. The caller stops rendering it. */
  onClose: () => void;
  children: ReactNode;
}

/** Heartwood's alert Dialog with Cancel and one commit: removing or stopping something is asked first. */
export function ConfirmDialog({ title, commit, danger, icon, isDisabled, isPending, onConfirm, keepOpen, onClose, children }: ConfirmDialogProps) {
  return (
    <Dialog
      title={title}
      size="sm"
      role="alertdialog"
      icon={icon}
      onClose={onClose}
      footer={(close) => (
        <>
          <Button variant="secondary" onPress={close}>
            Cancel
          </Button>
          <Button
            variant={danger ? 'destructive' : 'primary'}
            isDisabled={isDisabled}
            isPending={isPending}
            onPress={() => {
              onConfirm();
              if (!keepOpen) close();
            }}
          >
            {commit}
          </Button>
        </>
      )}
    >
      {children}
    </Dialog>
  );
}
