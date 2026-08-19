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
          amber: '#f59e0b',
          'amber-deep': '#d97706',
          'amber-light': '#fbbf24',
          'amber-text': '#fef3c7',
        },
        mineral: {
          cyan: '#0ea5e9',
          emerald: '#10b981',
          ruby: '#ef4444',
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
