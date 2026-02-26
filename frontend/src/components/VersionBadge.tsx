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
      className="fixed bottom-3 right-4 text-muted-foreground/70 select-none pointer-events-none"
      style={{ fontSize: 12 }}
    >
      v{version}
    </span>
  );
};

export default VersionBadge;
