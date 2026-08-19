/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        basalt: {
          void: '#090a0d',
          chassis: '#0f1115',
          card: '#15171d',
          'card-hover': '#1b1e26',
          border: '#23262f',
          bezel: '#343846',
        },
        quartz: {
          amber: '#fde047', // Soft Champagne Amber (Lighter & subtler)
          'amber-muted': 'rgba(253, 224, 71, 0.12)',
          'amber-border': 'rgba(253, 224, 71, 0.25)',
          'amber-soft': '#fef08a',
          'amber-text': '#fef9c3',
        },
        mineral: {
          cyan: '#38bdf8',
          emerald: '#34d399',
          ruby: '#f87171',
        },
      },
      fontFamily: {
        sans: ['Inter', '-apple-system', 'BlinkMacSystemFont', 'Segoe UI', 'Roboto', 'sans-serif'],
        mono: ['JetBrains Mono', 'Fira Code', 'Menlo', 'monospace'],
      },
    },
  },
  plugins: [],
}
