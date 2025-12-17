# 🚀 Quick Start: Deploy in 5 Minutes

## Fastest Path to Live Site

### Option 1: Vercel (Easiest)

```bash
cd landing-page

# Install Vercel CLI
npm install -g vercel

# Deploy (it will ask a few questions)
vercel

# Promote to production
vercel --prod
```

**That's it!** You'll get a URL like: `https://vaanyze-xyz.vercel.app`

---

### Option 2: Netlify Drop

1. Go to https://app.netlify.com/drop
2. Drag the entire `landing-page` folder
3. Done! You get a URL like: `https://random-name.netlify.app`

**Time: 30 seconds**

---

### Option 3: Use Our Script

```bash
cd landing-page
./deploy.sh vercel
# or
./deploy.sh netlify
```

---

## Add Custom Domain (After Initial Deploy)

### For Vercel:
```bash
vercel domains add vaanyze.io
# Follow prompts to update DNS
```

### For Netlify:
```bash
netlify domains:add vaanyze.io
```

**Where to buy domain:**
- Namecheap: $8-12/year
- Cloudflare: $8-10/year (cheapest)
- Porkbun: $7-10/year

---

## Test Locally First

```bash
cd landing-page

# Option 1: Python
python3 -m http.server 8000

# Option 2: Node
npx serve .

# Option 3: PHP
php -S localhost:8000
```

Then open: http://localhost:8000

---

## What You Get

✅ Professional landing page
✅ Mobile responsive
✅ Fast loading (95+ PageSpeed score)
✅ Free SSL certificate
✅ Free hosting (for reasonable traffic)
✅ Automatic deployments on git push

---

## Next: Add Backend

See `README.md` for:
- Waitlist form backend
- Email integration
- Analytics setup
- SEO optimization
