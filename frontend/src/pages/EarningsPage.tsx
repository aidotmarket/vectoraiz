import { useEffect, useState } from "react";
import { DollarSign, Wallet, TrendingUp, ShoppingCart, CreditCard, Activity } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import EarningsSkeleton from "@/components/skeletons/EarningsSkeleton";
import { useMarketplace } from "@/contexts/MarketplaceContext";
import { useMode } from "@/contexts/ModeContext";

const isSavedNumber = (value: unknown): value is number =>
  typeof value === "number" && Number.isFinite(value);

const formatMoney = (value: unknown) => isSavedNumber(value)
  ? `$${value.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`
  : "Unavailable";

const formatCount = (value: unknown) => isSavedNumber(value) ? value.toLocaleString() : "Unavailable";

// An incomplete set of saved figures cannot supply a total.
const savedTotal = (values: unknown[]) => values.length > 0 && values.every(isSavedNumber)
  ? values.reduce<number>((sum, value) => sum + (value as number), 0)
  : undefined;

const EarningsPage = () => {
  const [isLoading, setIsLoading] = useState(true);
  const { hasFeature } = useMode();

  const { publishedDatasets, unpublishDataset } = useMarketplace();

  // Allow the initial saved-record view to settle before displaying it.
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

  const totalEarnings = savedTotal(publishedDatasetsArray.map(d => d.earnings));
  const totalSales = savedTotal(publishedDatasetsArray.map(d => d.sales));

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
      <div>
        <h1 className="text-2xl font-semibold text-foreground">Saved listing figures</h1>
        <p className="text-sm text-muted-foreground">Figures and listing records saved on this device.</p>
      </div>
      {/* Overview Cards */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        <Card className="bg-card border-border">
          <CardContent className="p-6">
            <div className="flex items-center justify-between">
              <div>
                <p className="text-sm text-muted-foreground">Saved Earnings</p>
                <p className="text-3xl font-bold text-[hsl(var(--haven-success))]">
                  {formatMoney(totalEarnings)}
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
                  Unavailable
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
                  Unavailable
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
                <p className="text-sm text-muted-foreground">Saved Sales</p>
                <p className="text-3xl font-bold text-foreground">{formatCount(totalSales)}</p>
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
          <CardDescription>Dated earnings history is unavailable on this device.</CardDescription>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">Unavailable — saved listing figures do not include dated sales records.</p>
        </CardContent>
      </Card>

      {/* Published Datasets Section */}
      <div className="space-y-4">
        <h2 className="text-xl font-semibold text-foreground">Saved listing records</h2>
        {publishedDatasetsArray.length > 0 ? (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
            {publishedDatasetsArray.map((dataset) => (
              <Card key={dataset.id} className="bg-card border-border">
                <CardContent className="p-4 space-y-4">
                  <div>
                    <h3 className="font-semibold text-foreground truncate">{dataset.name}</h3>
                    <p className="text-2xl font-bold text-primary mt-1">{formatMoney(dataset.price)}</p>
                  </div>

                  <div className="grid grid-cols-3 gap-2 text-center">
                    <div className="p-2 bg-secondary/50 rounded-lg">
                      <p className="text-lg font-semibold text-foreground">{formatCount(dataset.sales)}</p>
                      <p className="text-xs text-muted-foreground">Sales</p>
                    </div>
                    <div className="p-2 bg-secondary/50 rounded-lg">
                      <p className="text-lg font-semibold text-[hsl(var(--haven-success))]">{formatMoney(dataset.earnings)}</p>
                      <p className="text-xs text-muted-foreground">Earnings</p>
                    </div>
                    <div className="p-2 bg-secondary/50 rounded-lg">
                      <p className="text-lg font-semibold text-foreground">{formatCount(dataset.views)}</p>
                      <p className="text-xs text-muted-foreground">Views</p>
                    </div>
                  </div>

                  <div className="space-y-2">
                    <p className="text-xs text-muted-foreground">Price editing and detailed stats are unavailable here.</p>
                    <Button
                      variant="outline"
                      size="sm"
                      aria-label={`Remove saved record for ${dataset.name} from this device`}
                      onClick={() => unpublishDataset(dataset.id)}
                    >
                      Remove saved record
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
                <p className="text-muted-foreground">No listing records saved on this device</p>
                <p className="text-sm text-muted-foreground/70">
                  Current listings and account information are available at ai.market.
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
                <CardDescription>Current account information is managed in ai.market.</CardDescription>
              </div>
            </div>
          </CardHeader>
          <CardContent className="space-y-4">
            <p className="text-sm text-muted-foreground">Account connection and payout settings: Unavailable here.</p>
            <a className="text-sm text-primary underline" href="https://ai.market/">
              Open ai.market for current account information
            </a>
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
                <CardDescription>Live marketplace API status is unavailable here.</CardDescription>
              </div>
            </div>
          </CardHeader>
          <CardContent>
            <p className="text-sm text-muted-foreground">Unavailable — no API health observations are available.</p>
          </CardContent>
        </Card>
      </div>
    </div>
  );
};

export default EarningsPage;
