import { MessageCircle } from "lucide-react";
import { useCoPilot } from "@/contexts/CoPilotContext";

export default function CoPilotFab() {
  const { toggle, isOpen, isStandalone, allieAvailable } = useCoPilot();

  if (isStandalone && !allieAvailable) return null;
  if (isOpen) return null;

  return (
    <button
      onClick={toggle}
      className="fixed bottom-5 right-5 z-40 h-12 w-12 rounded-full flex items-center justify-center shadow-lg transition-all hover:scale-105 active:scale-95 border border-white/[0.1]"
      style={{
        background: "rgba(12, 17, 30, 0.7)",
        backdropFilter: "blur(16px)",
        WebkitBackdropFilter: "blur(16px)",
      }}
      title="Open allAI"
    >
      <MessageCircle className="h-5 w-5 text-cyan-400/70" />
    </button>
  );
}
