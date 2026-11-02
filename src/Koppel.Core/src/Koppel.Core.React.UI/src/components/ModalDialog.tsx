import type { ReactNode } from 'react';
import { Dialog, Heading, Modal, ModalOverlay } from 'react-aria-components';

export interface ModalDialogProps {
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  children: ReactNode;
}

/**
 * A react-aria modal dialog: focus moves in and stays there, Escape closes it. A Button with
 * slot="close" inside closes it too.
 */
export function ModalDialog({ isOpen, onOpenChange, title, children }: ModalDialogProps) {
  return (
    <ModalOverlay className="kp-overlay" data-placement="center" isOpen={isOpen} onOpenChange={onOpenChange} isDismissable>
      <Modal>
        <Dialog className="kp-dialog">
          <Heading slot="title">{title}</Heading>
          {children}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
