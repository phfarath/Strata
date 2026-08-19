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
        background: '#09090b', // Zinc 950
        surface: {
          50: '#18181b', // Zinc 900
          100: '#27272a', // Zinc 800
          200: '#3f3f46', // Zinc 700
          border: '#27272a',
          'border-hover': '#3f3f46',
        },
        accent: {
          DEFAULT: '#3b82f6', // Clean Blue
          hover: '#2563eb',
          muted: '#1e3a8a',
        },
        success: '#10b981',
        danger: '#ef4444',
        warning: '#f59e0b',
      },
      fontFamily: {
        sans: ['Inter', '-apple-system', 'BlinkMacSystemFont', 'Segoe UI', 'Roboto', 'sans-serif'],
        mono: ['JetBrains Mono', 'Fira Code', 'Menlo', 'monospace'],
      },
    },
  },
  plugins: [],
}
