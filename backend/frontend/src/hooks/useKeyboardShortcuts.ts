import { useEffect } from "react";
import { useNavigate } from "react-router-dom";

interface UseKeyboardShortcutsOptions {
  onCommandPalette: () => void;
  onUploadModal: () => void;
  onHelpModal: () => void;
}

const useKeyboardShortcuts = ({
  onCommandPalette,
  onUploadModal,
  onHelpModal,
}: UseKeyboardShortcutsOptions) => {
  const navigate = useNavigate();

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      // Ignore if user is typing in an input
      const target = e.target as HTMLElement;
      if (
        target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.isContentEditable
      ) {
        return;
      }

      // Cmd/Ctrl + K - Open command palette
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        onCommandPalette();
        return;
      }

      // Cmd/Ctrl + U - Open upload modal
      if ((e.metaKey || e.ctrlKey) && e.key === "u") {
        e.preventDefault();
        onUploadModal();
        return;
      }

      // ? - Show help modal
      if (e.key === "?" && !e.metaKey && !e.ctrlKey) {
        e.preventDefault();
        onHelpModal();
        return;
      }

      // G + D - Go to Dashboard
      if (e.key === "d" && !e.metaKey && !e.ctrlKey) {
        // Simple navigation shortcuts
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [navigate, onCommandPalette, onUploadModal, onHelpModal]);
};

export default useKeyboardShortcuts;
