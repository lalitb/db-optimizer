#!/bin/bash

# Quick deployment script for Vaanyze landing page
# Usage: ./deploy.sh [platform]
# Platforms: vercel (default), netlify, github

PLATFORM=${1:-vercel}

echo "🚀 Deploying Vaanyze landing page to $PLATFORM..."

case $PLATFORM in
  vercel)
    echo "📦 Installing Vercel CLI (if needed)..."
    if ! command -v vercel &> /dev/null; then
        npm install -g vercel
    fi

    echo "🚢 Deploying to Vercel..."
    vercel --prod

    echo "✅ Deployed! Your site is live."
    echo "💡 Tip: Run 'vercel domains add yourdomain.com' to add a custom domain"
    ;;

  netlify)
    echo "📦 Installing Netlify CLI (if needed)..."
    if ! command -v netlify &> /dev/null; then
        npm install -g netlify-cli
    fi

    echo "🚢 Deploying to Netlify..."
    netlify deploy --prod --dir=.

    echo "✅ Deployed! Your site is live."
    ;;

  github)
    echo "📦 Deploying to GitHub Pages..."
    echo "⚠️  Make sure you have a repo set up: your-username.github.io"

    read -p "Enter your GitHub username: " username

    git init
    git add .
    git commit -m "Deploy landing page"
    git branch -M main
    git remote add origin https://github.com/$username/vaanyze.github.io.git
    git push -u origin main

    echo "✅ Pushed to GitHub!"
    echo "💡 Enable GitHub Pages in your repo settings"
    echo "   Settings → Pages → Source: main branch"
    ;;

  *)
    echo "❌ Unknown platform: $PLATFORM"
    echo "Usage: ./deploy.sh [vercel|netlify|github]"
    exit 1
    ;;
esac
