/**
 * useChannel — Returns the current channel ("direct" | "marketplace" | "aim-data").
 *
 * Channel is a presentation hint from VECTORAIZ_CHANNEL env var.
 * It affects sidebar ordering, allAI greeting, and default landing page.
 * It NEVER gates features, auth, or billing (Condition C2).
 *
 * BQ-VZ-CHANNEL
 */
import { useMode } from "@/contexts/ModeContext";

export type Channel = "direct" | "marketplace" | "aim-data";

export function useChannel(): Channel {
  const { channel } = useMode();
  return channel;
}
