import { useEffect, useState } from "react";
import {
  DollarSign,
  Wallet,
  TrendingUp,
  ShoppingCart,
  Edit3,
  BarChart3,
  Power,
  CheckCircle,
  Clock,
  ChevronLeft,
  ChevronRight,
  CreditCard,
  Activity,
  Settings,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip as RechartsTooltip,
  ResponsiveContainer,
  BarChart,
  Bar,
} from "recharts";
import EarningsSkeleton from "@/components/skeletons/EarningsSkeleton";
import { useMarketplace, type PublishedDataset } from "@/contexts/MarketplaceContext";
import { useMode } from "@/contexts/ModeContext";

// Generate earnings chart data from published datasets
const generateEarningsChartData = (publishedDatasets: Record<string, PublishedDataset>) => {
  const data = [];
  const today = new Date();
  const datasets = Object.values(publishedDatasets);
  
  for (let i = 29; i >= 0; i--) {
    const date = new Date(today);
    date.setDate(date.getDate() - i);
    
    // Calculate earnings for this day based on published datasets
    let dayEarnings = 0;
    datasets.forEach(dataset => {
      const publishedDate = new Date(dataset.publishedAt);
      if (publishedDate <= date) {
        // Simple distribution of earnings across days since publish
        const daysSincePublish = Math.floor((date.getTime() - publishedDate.getTime()) / (1000 * 60 * 60 * 24));
        if (daysSincePublish >= 0 && dataset.purchases > 0) {
          dayEarnings += Math.round((dataset.earnings / Math.max(1, daysSincePublish + 1)) * 0.1);
        }
      }
    });
    
    data.push({
      date: date.toLocaleDateString("en-US", { month: "short", day: "numeric" }),
      fullDate: date.toISOString().split("T")[0],
      earnings: dayEarnings,
    });
  }
  return data;
};

// Generate uptime data based on days of the week
const generateUptimeData = () => {
  const days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  const today = new Date().getDay();
  const data = [];
  for (let i = 6; i >= 0; i--) {
    const dayIndex = (today - i + 7) % 7;
    data.push({ day: days[dayIndex], uptime: 100 });
  }
  return data;
};

const uptimeData = generateUptimeData();

const EarningsPage = () => {
  const [isLoading, setIsLoading] = useState(true);
  const [currentPage, setCurrentPage] = useState(1);
  const rowsPerPage = 5;
  const { hasFeature } = useMode();

  const { publishedDatasets, getTotalEarnings, getPublishedCount, unpublishDataset } = useMarketplace();

  // Simulate loading
  useEffect(() => {
    const timer = setTimeout(() => setIsLoading(false), 300);
    return () => clearTimeout(timer);
  }, []);

  // Convert published datasets to array for display
  const publishedDatasetsArray = Object.values(publishedDatasets).map(d => ({
    id: d.id,
    name: d.title,
    price: d.price,
    sales: d.purchases,
    earnings: d.earnings,
    views: d.views,
    publishedAt: d.publishedAt,
  }));

  // Generate chart data from real published datasets
  const earningsChartData = generateEarningsChartData(publishedDatasets);

  // Overview stats from MarketplaceContext
  const totalEarnings = getTotalEarnings();
  const totalSales = publishedDatasetsArray.reduce((sum, d) => sum + d.sales, 0);
  const pendingPayout = Math.round(totalEarnings * 0.15); // 15% pending
  const thisMonth = Math.round(totalEarnings * 0.25); // Approximate this month

  if (!hasFeature("marketplace")) {
    return (
      <div className="flex flex-col items-center justify-center py-24 space-y-4">
        <div className="w-16 h-16 rounded-full bg-secondary flex items-center justify-center">
          <DollarSign className="w-8 h-8 text-muted-foreground" />
        </div>
        <h2 className="text-xl font-semibold text-foreground">Marketplace Not Available</h2>
        <p className="text-muted-foreground text-center max-w-md">
          Earnings and marketplace features require connected mode. Connect your instance to the marketplace to start selling datasets.
        </p>
      </div>
    );
  }

  if (isLoading) {
    return <EarningsSkeleton />;
  }

  return (
    <div className="space-y-6 animate-in fade-in duration-300">
      {/* Overview Cards */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        <Card className="bg-card border-border">
          <CardContent className="p-6">
            <div className="flex items-center justify-between">
              <div>
                <p className="text-sm text-muted-foreground">Total Earnings</p>
                <p className="text-3xl font-bold text-[hsl(var(--haven-success))]">
                  ${totalEarnings.toLocaleString()}.00
                </p>
              </div>
              <div className="w-12 h-12 rounded-lg bg-[hsl(var(--haven-success))]/20 flex items-center justify-center">
                <DollarSign className="w-6 h-6 text-[hsl(var(--haven-success))]" />
              </div>
            </div>
          </CardContent>
        </Card>

        <Card className="bg-card border-border">
          <CardContent className="p-6">
            <div className="flex items-center justify-between">
              <div>
                <p className="text-sm text-muted-foreground">Pending Payout</p>
                <p className="text-3xl font-bold text-[hsl(var(--haven-warning))]">
                  ${pendingPayout.toLocaleString()}.00
                </p>
              </div>
              <div className="w-12 h-12 rounded-lg bg-[hsl(var(--haven-warning))]/20 flex items-center justify-center">
                <Wallet className="w-6 h-6 text-[hsl(var(--haven-warning))]" />
              </div>
            </div>
          </CardContent>
        </Card>

        <Card className="bg-card border-border">
          <CardContent className="p-6">
            <div className="flex items-center justify-between">
              <div>
                <p className="text-sm text-muted-foreground">This Month</p>
                <p className="text-3xl font-bold text-primary">
                  ${thisMonth.toLocaleString()}.00
                </p>
              </div>
              <div className="w-12 h-12 rounded-lg bg-primary/20 flex items-center justify-center">
                <TrendingUp className="w-6 h-6 text-primary" />
              </div>
            </div>
          </CardContent>
        </Card>

        <Card className="bg-card border-border">
          <CardContent className="p-6">
            <div className="flex items-center justify-between">
              <div>
                <p className="text-sm text-muted-foreground">Datasets Sold</p>
                <p className="text-3xl font-bold text-foreground">{totalSales}</p>
              </div>
              <div className="w-12 h-12 rounded-lg bg-secondary flex items-center justify-center">
                <ShoppingCart className="w-6 h-6 text-muted-foreground" />
              </div>
            </div>
          </CardContent>
        </Card>
      </div>

      {/* Earnings Chart */}
      <Card className="bg-card border-border">
        <CardHeader>
          <CardTitle className="text-foreground">Earnings Over Time</CardTitle>
          <CardDescription>Your earnings for the last 30 days</CardDescription>
        </CardHeader>
        <CardContent>
          <div className="h-[300px]">
            <ResponsiveContainer width="100%" height="100%">
              <LineChart data={earningsChartData}>
                <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
                <XAxis 
                  dataKey="date" 
                  stroke="hsl(var(--muted-foreground))"
                  tick={{ fill: "hsl(var(--muted-foreground))", fontSize: 12 }}
                  tickLine={{ stroke: "hsl(var(--border))" }}
                />
                <YAxis 
                  stroke="hsl(var(--muted-foreground))"
                  tick={{ fill: "hsl(var(--muted-foreground))", fontSize: 12 }}
                  tickLine={{ stroke: "hsl(var(--border))" }}
                  tickFormatter={(value) => `$${value}`}
                />
                <RechartsTooltip 
                  contentStyle={{
                    backgroundColor: "hsl(var(--card))",
                    border: "1px solid hsl(var(--border))",
                    borderRadius: "8px",
                    color: "hsl(var(--foreground))",
                  }}
                  labelStyle={{ color: "hsl(var(--muted-foreground))" }}
                  formatter={(value: number) => [`$${value}`, "Earnings"]}
                />
                <Line 
                  type="monotone" 
                  dataKey="earnings" 
                  stroke="hsl(var(--primary))" 
                  strokeWidth={2}
                  dot={false}
                  activeDot={{ r: 6, fill: "hsl(var(--primary))" }}
                />
              </LineChart>
            </ResponsiveContainer>
          </div>
        </CardContent>
      </Card>

      {/* Published Datasets Section */}
      <div className="space-y-4">
        <h2 className="text-xl font-semibold text-foreground">Your Published Datasets</h2>
        {publishedDatasetsArray.length > 0 ? (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
            {publishedDatasetsArray.map((dataset) => (
              <Card key={dataset.id} className="bg-card border-border">
                <CardContent className="p-4 space-y-4">
                  <div>
                    <h3 className="font-semibold text-foreground truncate">{dataset.name}</h3>
                    <p className="text-2xl font-bold text-primary mt-1">${dataset.price}</p>
                  </div>

                  <div className="grid grid-cols-3 gap-2 text-center">
                    <div className="p-2 bg-secondary/50 rounded-lg">
                      <p className="text-lg font-semibold text-foreground">{dataset.sales}</p>
                      <p className="text-xs text-muted-foreground">Sales</p>
                    </div>
                    <div className="p-2 bg-secondary/50 rounded-lg">
                      <p className="text-lg font-semibold text-[hsl(var(--haven-success))]">${dataset.earnings}</p>
                      <p className="text-xs text-muted-foreground">Earnings</p>
                    </div>
                    <div className="p-2 bg-secondary/50 rounded-lg">
                      <p className="text-lg font-semibold text-foreground">{dataset.views}</p>
                      <p className="text-xs text-muted-foreground">Views</p>
                    </div>
                  </div>

                  <div className="flex items-center gap-2">
                    <Button variant="outline" size="sm" className="flex-1 gap-1">
                      <Edit3 className="w-3 h-3" />
                      Edit Price
                    </Button>
                    <Button variant="outline" size="sm" className="flex-1 gap-1">
                      <BarChart3 className="w-3 h-3" />
                      Stats
                    </Button>
                    <Button 
                      variant="ghost" 
                      size="sm" 
                      className="text-destructive hover:text-destructive"
                      onClick={() => unpublishDataset(dataset.id)}
                    >
                      <Power className="w-3 h-3" />
                    </Button>
                  </div>
                </CardContent>
              </Card>
            ))}
          </div>
        ) : (
          <Card className="bg-card border-border border-dashed">
            <CardContent className="py-12">
              <div className="text-center space-y-3">
                <div className="w-12 h-12 rounded-full bg-secondary flex items-center justify-center mx-auto">
                  <ShoppingCart className="w-6 h-6 text-muted-foreground" />
                </div>
                <p className="text-muted-foreground">No published datasets yet</p>
                <p className="text-sm text-muted-foreground/70">
                  Publish a dataset to start earning
                </p>
              </div>
            </CardContent>
          </Card>
        )}
      </div>

      {/* Bottom Cards Row */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {/* Payout Settings */}
        <Card className="bg-card border-border">
          <CardHeader>
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded-lg bg-secondary flex items-center justify-center">
                <CreditCard className="w-5 h-5 text-primary" />
              </div>
              <div>
                <CardTitle className="text-foreground">Payout Settings</CardTitle>
                <CardDescription>Manage your payment method</CardDescription>
              </div>
            </div>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex items-center justify-between p-3 bg-secondary/50 rounded-lg">
              <div className="flex items-center gap-3">
                <div className="w-8 h-8 bg-[#635BFF] rounded flex items-center justify-center">
                  <span className="text-white text-xs font-bold">S</span>
                </div>
                <div>
                  <p className="text-sm font-medium text-foreground">Stripe account</p>
                  <p className="text-xs text-muted-foreground">Not connected</p>
                </div>
              </div>
              <Button variant="outline" size="sm">Connect</Button>
            </div>
            <Button variant="ghost" size="sm" className="w-full gap-2">
              <Settings className="w-4 h-4" />
              Manage Payout Settings
            </Button>
          </CardContent>
        </Card>

        {/* API Status */}
        <Card className="bg-card border-border">
          <CardHeader>
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded-lg bg-secondary flex items-center justify-center">
                <Activity className="w-5 h-5 text-primary" />
              </div>
              <div>
                <CardTitle className="text-foreground">API Status</CardTitle>
                <CardDescription>Your marketplace API health</CardDescription>
              </div>
            </div>
          </CardHeader>
          <CardContent>
            <div className="h-[100px]">
              <ResponsiveContainer width="100%" height="100%">
                <BarChart data={uptimeData}>
                  <Bar 
                    dataKey="uptime" 
                    fill="hsl(var(--haven-success))" 
                    radius={[4, 4, 0, 0]}
                  />
                </BarChart>
              </ResponsiveContainer>
            </div>
            <div className="flex items-center justify-between mt-2">
              <span className="text-xs text-muted-foreground">Last 7 days</span>
              <Badge variant="secondary" className="bg-[hsl(var(--haven-success))]/20 text-[hsl(var(--haven-success))] border-[hsl(var(--haven-success))]/30">
                <CheckCircle className="w-3 h-3 mr-1" />
                100% Uptime
              </Badge>
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  );
};

export default EarningsPage;
