# Landing Page Deployment Guide

## 🚀 Hosting Options (Ranked by Ease)

### Option 1: Vercel (RECOMMENDED) ⭐
**Best for: Quick deployment, free tier, automatic HTTPS**

```bash
# Install Vercel CLI
npm install -g vercel

# Deploy (from landing-page directory)
cd landing-page
vercel

# Follow prompts - it will:
# 1. Create a new project
# 2. Deploy to https://your-project.vercel.app
# 3. Give you a production URL in <30 seconds
```

**Cost:** FREE for personal projects
**Custom domain:** $15/year for domain, free SSL
**Time to deploy:** 2 minutes

### Option 2: Netlify
**Best for: Drag-and-drop deployment, free forms**

```bash
# Install Netlify CLI
npm install -g netlify-cli

# Deploy
cd landing-page
netlify deploy

# Or just drag-and-drop the folder at netlify.com
```

**Cost:** FREE
**Features:** Free forms (for waitlist), free SSL
**Time to deploy:** 3 minutes

### Option 3: GitHub Pages
**Best for: Already using GitHub, completely free**

```bash
# Create a new repo: your-username.github.io
# Push landing-page/index.html as index.html

git init
git add index.html
git commit -m "Initial landing page"
git remote add origin https://github.com/your-username/vaanyze.github.io.git
git push -u origin main

# Enable GitHub Pages in repo settings
# Will be live at: https://your-username.github.io
```

**Cost:** FREE
**Limitation:** No server-side code (but our landing page is static anyway)

### Option 4: Azure Static Web Apps
**Best for: You're at Microsoft, seamless integration**

```bash
# Install Azure CLI
brew install azure-cli
az login

# Create static web app
az staticwebapp create \
  --name vaanyze \
  --resource-group your-rg \
  --source ./landing-page \
  --location westus2

# Deploys to: https://vaanyze.azurestaticapps.net
```

**Cost:** FREE tier available
**Advantage:** Azure integration, can connect to Azure Functions for backend

---

## 📧 Backend Integration

### Quick Setup: Vercel + Supabase (Easiest)

1. **Deploy frontend to Vercel** (as above)

2. **Create Supabase project** (free tier)
```bash
# Sign up at supabase.com
# Create new project
# Create 'waitlist' table:

CREATE TABLE waitlist (
  id SERIAL PRIMARY KEY,
  email TEXT UNIQUE NOT NULL,
  created_at TIMESTAMP DEFAULT NOW()
);
```

3. **Add API route** (Vercel Serverless Function)

Create `landing-page/api/waitlist.js`:
```javascript
import { createClient } from '@supabase/supabase-js';

const supabase = createClient(
  process.env.SUPABASE_URL,
  process.env.SUPABASE_ANON_KEY
);

export default async function handler(req, res) {
  if (req.method !== 'POST') {
    return res.status(405).json({ error: 'Method not allowed' });
  }

  const { email } = req.body;

  const { data, error } = await supabase
    .from('waitlist')
    .insert([{ email }]);

  if (error) {
    return res.status(400).json({ error: error.message });
  }

  return res.status(200).json({ success: true });
}
```

4. **Add environment variables to Vercel**
```bash
vercel env add SUPABASE_URL
vercel env add SUPABASE_ANON_KEY
```

**Total cost:** FREE for <500 signups/month

---

## 🎨 Should You Use Lovable/V0/Framer vs Custom?

### Use NO-CODE Tools (Lovable, Framer, Webflow) If:
✅ You want to iterate on design quickly
✅ You're not a frontend developer
✅ You need a marketing site only (no backend integration)
✅ Budget is tight (<$1000/month)

**Recommended: Framer**
- Beautiful templates
- Easy to customize
- Built-in forms
- $20/month for custom domain

### Use CUSTOM HTML (What I Built) If:
✅ You want full control
✅ You'll integrate with your Rust backend
✅ You want to avoid monthly fees
✅ You're technical enough to deploy to Vercel

**Our landing page:**
- 100% custom code
- No dependencies (just Tailwind CDN)
- Free to host on Vercel/Netlify
- Easy to modify
- Can connect to your existing db-optimizer API

---

## 🔧 Connecting to Your Rust Backend

### Option A: Use Vaanyze HTTP API

Your code already has `src/http_server.rs`. Extend it:

```rust
// Add to src/http/handlers/waitlist.rs
use axum::{Json, http::StatusCode};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Deserialize)]
pub struct WaitlistRequest {
    email: String,
}

#[derive(Serialize)]
pub struct WaitlistResponse {
    success: bool,
}

pub async fn add_to_waitlist(
    pool: Extension<PgPool>,
    Json(req): Json<WaitlistRequest>,
) -> Result<Json<WaitlistResponse>, StatusCode> {
    sqlx::query!(
        "INSERT INTO waitlist (email) VALUES ($1) ON CONFLICT (email) DO NOTHING",
        req.email
    )
    .execute(pool.0.as_ref())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(WaitlistResponse { success: true }))
}
```

Then in landing page, update form handler:
```javascript
const response = await fetch('https://your-api.com/api/waitlist', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email })
});
```

### Option B: Separate Landing Page Backend (Simpler for now)

Use Vercel Functions + Supabase (shown above)
- Keep landing page separate from main app
- Easier to iterate on marketing site
- Can migrate to main backend later

---

## 🎯 Recommended Stack for You

**Based on your situation (Microsoft, technical, Rust backend):**

```
Landing Page:   Vercel (FREE)
                - Deploy custom HTML (what I built)
                - Custom domain: vaanyze.io

Waitlist:       Supabase (FREE tier)
                - Postgres database
                - REST API auto-generated

Email:          Resend (FREE tier)
                - Send welcome emails
                - API integration

Analytics:      Plausible or PostHog (FREE tier)
                - Privacy-friendly
                - No cookies needed
```

**Total monthly cost: $0** (until you hit limits)

---

## 📊 Adding Analytics

### Plausible (Recommended - Privacy-Friendly)

1. Sign up at plausible.io (or self-host)
2. Add script before `</head>`:
```html
<script defer data-domain="yourdomain.com" src="https://plausible.io/js/script.js"></script>
```

3. See pageviews, referrers, conversions

**Cost:** $9/month (or FREE self-hosted)

---

## 🚢 Deployment Checklist

- [ ] Deploy to Vercel/Netlify
- [ ] Set up custom domain
- [ ] Connect waitlist backend (Supabase)
- [ ] Add analytics (Plausible)
- [ ] Test form submission
- [ ] Set up email notifications (Resend)
- [ ] Add SEO meta tags
- [ ] Test on mobile
- [ ] Set up monitoring (UptimeRobot - free)

---

## 🎨 Customization Guide

### Change Colors
Find this in index.html:
```css
.gradient-bg {
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
}
```
Replace with your brand colors

### Update Pricing
Search for `id="pricing"` section and modify:
```html
<span class="text-5xl font-bold">$299</span>
```

### Add Testimonials
Add this section after features:
```html
<section class="py-20 px-4">
    <div class="max-w-7xl mx-auto">
        <!-- Add customer quotes here -->
    </div>
</section>
```

---

## 📈 Next Steps After Launch

1. **Week 1:** Get 10 beta users
   - Share on Twitter, LinkedIn, HackerNews
   - Post in r/PostgreSQL

2. **Week 2:** Collect feedback
   - Schedule calls with beta users
   - Fix obvious issues

3. **Week 3:** Add social proof
   - Case studies
   - Testimonials
   - Usage metrics

4. **Month 2:** SEO & Content
   - Blog posts on eBPF
   - Technical deep-dives
   - Comparison pages

---

## 🔗 Resources

- **Vercel Docs:** https://vercel.com/docs
- **Tailwind CSS:** https://tailwindcss.com/docs
- **Supabase:** https://supabase.com/docs
- **Domain purchase:** Namecheap, Cloudflare ($8-15/year)

**Recommended domain:** `vaanyze.io` (check availability on Namecheap)
