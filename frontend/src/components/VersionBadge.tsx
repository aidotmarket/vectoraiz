import { useEffect, useState } from "react";
import { getApiUrl } from "@/lib/api";

const VersionBadge = () => {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    fetch(`${getApiUrl()}/api/health`)
      .then((r) => (r.ok ? r.json() : null))
      .then((data) => {
        if (data?.version) setVersion(data.version);
      })
      .catch(() => {});
  }, []);

  if (!version) return null;

  // Only render badge for real release versions (semver-like).
  // Suppresses placeholder values like "latest", "dev", "unknown", "0.0.0".
  const normalized = version.replace(/^v/i, "");
  const isReleaseVersion = /^\d+\.\d+\.\d+/.test(normalized) && !/^0\.0\.0$/.test(normalized);
  if (!isReleaseVersion) return null;

  return (
    <span
      className="fixed bottom-3 right-4 text-muted-foreground/70 select-none pointer-events-none"
      style={{ fontSize: 12 }}
    >
      v{normalized}
    </span>
  );
};

export default VersionBadge;
