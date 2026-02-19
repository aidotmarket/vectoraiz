import { useState } from "react";
import { Outlet } from "react-router-dom";
import Sidebar from "./Sidebar";
import TopBar from "./TopBar";
import { cn } from "@/lib/utils";
import CommandPalette from "@/components/CommandPalette";
import KeyboardShortcutsModal from "@/components/KeyboardShortcutsModal";
import FileUploadModal from "@/components/FileUploadModal";
import useKeyboardShortcuts from "@/hooks/useKeyboardShortcuts";
import { useToast } from "@/hooks/use-toast";

const MainLayout = () => {
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
  const [uploadModalOpen, setUploadModalOpen] = useState(false);
  const [helpModalOpen, setHelpModalOpen] = useState(false);
  const { toast } = useToast();

  useKeyboardShortcuts({
    onCommandPalette: () => setCommandPaletteOpen(true),
    onUploadModal: () => setUploadModalOpen(true),
    onHelpModal: () => setHelpModalOpen(true),
  });

  const handleUploadSuccess = () => {
    toast({
      title: "Dataset uploaded successfully",
      description: "Your file has been processed and is ready to use.",
    });
  };

  return (
    <div className="min-h-screen bg-background">
      <Sidebar 
        collapsed={sidebarCollapsed} 
        onToggle={() => setSidebarCollapsed(!sidebarCollapsed)} 
      />
      
      <div
        className={cn(
          "sidebar-transition",
          sidebarCollapsed ? "ml-[60px]" : "ml-[240px]"
        )}
      >
        <TopBar onOpenCommandPalette={() => setCommandPaletteOpen(true)} />
        <main className="p-6">
          <Outlet />
        </main>
      </div>

      {/* Global Modals */}
      <CommandPalette
        open={commandPaletteOpen}
        onOpenChange={setCommandPaletteOpen}
        onOpenUpload={() => setUploadModalOpen(true)}
      />
      <KeyboardShortcutsModal
        open={helpModalOpen}
        onOpenChange={setHelpModalOpen}
      />
      <FileUploadModal
        open={uploadModalOpen}
        onOpenChange={setUploadModalOpen}
        onSuccess={handleUploadSuccess}
      />
    </div>
  );
};

export default MainLayout;
