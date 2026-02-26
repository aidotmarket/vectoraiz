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

  return (
    <span
      className="fixed bottom-2 right-3 text-muted-foreground select-none pointer-events-none"
      style={{ fontSize: 11, opacity: 0.45 }}
    >
      v{version}
    </span>
  );
};

export default VersionBadge;
