import { useState } from "react";
import { Database, Rows3, HardDrive, Upload, Code, Clock, TrendingUp, DollarSign, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useToast } from "@/hooks/use-toast";
import FileUploadModal from "@/components/FileUploadModal";
import { useMarketplace } from "@/contexts/MarketplaceContext";
import { useDatasets } from "@/hooks/useApi";
import { useNavigate } from "react-router-dom";
import { useMode } from "@/contexts/ModeContext";

const Dashboard = () => {
  const [uploadModalOpen, setUploadModalOpen] = useState(false);
  const { toast } = useToast();
  const navigate = useNavigate();
  const { getPublishedCount, getTotalEarnings } = useMarketplace();
  const { data: datasetsData, loading: datasetsLoading } = useDatasets();
  const { hasFeature } = useMode();

  const publishedCount = getPublishedCount();
  const totalEarnings = getTotalEarnings();

  // Calculate stats from API data
  const datasets = datasetsData?.datasets || [];
  const totalDatasets = datasets.length;
  const totalRows = datasets.reduce((sum, d) => sum + (d.metadata?.row_count || 0), 0);
  const totalStorage = datasets.reduce((sum, d) => sum + (d.metadata?.size_bytes || 0), 0);

  const stats = [
    { label: "Total Datasets", value: totalDatasets.toString(), icon: Database, loading: datasetsLoading, link: "/datasets" },
    { label: "Total Rows Processed", value: totalRows.toLocaleString(), icon: Rows3, loading: datasetsLoading, link: "/datasets" },
    { label: "Storage Used", value: `${(totalStorage / (1024 * 1024)).toFixed(1)} MB`, icon: HardDrive, loading: datasetsLoading, link: "/datasets" },
    ...(hasFeature("marketplace") ? [
      { label: "Published", value: publishedCount.toString(), icon: TrendingUp, loading: false, link: "/datasets" },
      { label: "Marketplace Earnings", value: `$${totalEarnings.toLocaleString()}`, icon: DollarSign, loading: false, link: "/datasets" },
    ] : []),
  ];

  const handleUploadSuccess = () => {
    toast({
      title: "Dataset uploaded successfully",
      description: "Your file has been processed and is ready to use.",
    });
  };

  return (
    <div className="space-y-8">
      {/* Welcome Section */}
      <div className="space-y-2">
        <h2 className="text-3xl font-bold text-foreground">Welcome to vectorAIz</h2>
        <p className="text-muted-foreground">
          Your local Artificial Intelligence ingestion engine.
        </p>
      </div>

      {/* Stats Cards */}
      <div className="grid grid-cols-1 md:grid-cols-3 lg:grid-cols-5 gap-4">
        {stats.map((stat) => (
          <Card key={stat.label} className="card-hover bg-card border-border cursor-pointer" onClick={() => stat.link && navigate(stat.link)}>
            <CardHeader className="flex flex-row items-center justify-between pb-2">
              <CardTitle className="text-sm font-medium text-muted-foreground">
                {stat.label}
              </CardTitle>
              <stat.icon className="w-5 h-5 text-primary" />
            </CardHeader>
            <CardContent>
              {stat.loading ? (
                <Skeleton className="h-9 w-20" />
              ) : (
                <div className="text-3xl font-bold text-foreground">{stat.value}</div>
              )}
            </CardContent>
          </Card>
        ))}
      </div>

      {/* Quick Actions */}
      <div className="space-y-4">
        <h3 className="text-lg font-semibold text-foreground">Quick Actions</h3>
        <div className="flex flex-wrap gap-3">
          <Button className="gap-2" onClick={() => setUploadModalOpen(true)}>
            <Upload className="w-4 h-4" />
            Upload Dataset
          </Button>
          <Button variant="secondary" className="gap-2" onClick={() => navigate("/sql")}>
            <Code className="w-4 h-4" />
            New SQL Query
          </Button>
        </div>
      </div>

      {/* Recent Activity */}
      <div className="space-y-4">
        <h3 className="text-lg font-semibold text-foreground">Recent Activity</h3>
        <Card className="bg-card border-border">
          <CardContent className="py-12">
            <div className="flex flex-col items-center justify-center text-center space-y-3">
              <div className="w-12 h-12 rounded-full bg-secondary flex items-center justify-center">
                <Clock className="w-6 h-6 text-muted-foreground" />
              </div>
              <p className="text-muted-foreground">No recent activity</p>
              <p className="text-sm text-muted-foreground/70">
                Your recent actions will appear here
              </p>
            </div>
          </CardContent>
        </Card>
      </div>

      {/* Upload Modal */}
      <FileUploadModal
        open={uploadModalOpen}
        onOpenChange={setUploadModalOpen}
        onSuccess={handleUploadSuccess}
      />
    </div>
  );
};

export default Dashboard;
