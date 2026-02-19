#!/bin/bash
# vectoraiz-frontend Vercel Deployment Script
# Run this interactively on the Mac Studio
# Prerequisites: Vercel CLI installed (vercel --version)
set -e

echo "=== vectorAIz Frontend → Vercel Deployment ==="
echo ""

# Step 1: Auth check
echo "[1/7] Checking Vercel authentication..."
if ! vercel whoami 2>/dev/null; then
    echo "  → Not logged in. Opening browser for OAuth..."
    vercel login
fi
echo "  ✅ Authenticated as: $(vercel whoami)"
echo ""

# Step 2: Link project to GitHub repo
echo "[2/7] Linking project to Vercel..."
if [ ! -f ".vercel/project.json" ]; then
    vercel link --yes
else
    echo "  ✅ Already linked"
    cat .vercel/project.json
fi
echo ""

# Step 3: Set environment variables
echo "[3/7] Setting environment variables..."
echo "  Setting VITE_API_BASE_URL..."
echo "https://vectoraiz-backend-production.up.railway.app" | vercel env add VITE_API_BASE_URL production --force 2>/dev/null || \
    echo "https://vectoraiz-backend-production.up.railway.app" | vercel env add VITE_API_BASE_URL production

echo ""
echo "  ⚠️  VITE_GITHUB_TOKEN must be set manually if not already:"
echo "  Run: vercel env add VITE_GITHUB_TOKEN production"
echo "  Then paste your GitHub PAT (ghp_...)"
echo ""

# Step 4: Add custom domain
echo "[4/7] Adding custom domain vectoraiz.com..."
vercel domains add vectoraiz.com 2>/dev/null || echo "  Domain may already be added"
vercel domains add www.vectoraiz.com 2>/dev/null || echo "  www domain may already be added"
echo ""

# Step 5: Show DNS instructions
echo "[5/7] DNS Configuration Required at your registrar:"
echo "  ┌─────────────────────────────────────────────────┐"
echo "  │ Type   │ Name │ Value                           │"
echo "  ├─────────────────────────────────────────────────┤"
echo "  │ A      │ @    │ 76.76.21.21                     │"
echo "  │ CNAME  │ www  │ cname.vercel-dns.com            │"
echo "  └─────────────────────────────────────────────────┘"
echo "  (Vercel may also suggest using nameservers instead)"
echo ""

# Step 6: Deploy to production
echo "[6/7] Deploying to production..."
vercel --prod --yes
echo ""

# Step 7: Verify
echo "[7/7] Verifying deployment..."
echo "  Waiting 10s for propagation..."
sleep 10

VERCEL_URL=$(vercel inspect --json 2>/dev/null | grep -o '"url":"[^"]*"' | head -1 | cut -d'"' -f4)
echo "  Vercel URL: ${VERCEL_URL:-'check vercel dashboard'}"

echo ""
echo "  Testing HTTPS..."
curl -sI "https://vectoraiz.com" 2>/dev/null | head -5 || echo "  ⚠️  DNS may not have propagated yet (can take up to 48h)"

echo ""
echo "  Testing SSL certificate..."
echo | openssl s_client -connect vectoraiz.com:443 -servername vectoraiz.com 2>/dev/null | openssl x509 -noout -dates 2>/dev/null || echo "  ⚠️  SSL not ready yet (Vercel auto-provisions after DNS propagation)"

echo ""
echo "=== Deployment Complete ==="
echo ""
echo "Checklist:"
echo "  [ ] Vercel dashboard: https://vercel.com/dashboard"
echo "  [ ] DNS propagation: https://dnschecker.org/#A/vectoraiz.com"
echo "  [ ] SSL check: https://www.ssllabs.com/ssltest/analyze.html?d=vectoraiz.com"
echo "  [ ] Live site: https://vectoraiz.com"
