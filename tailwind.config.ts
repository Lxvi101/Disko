import type { Config } from 'tailwindcss';

export default {
  content: ['./index.html', './src/**/*.{js,ts,jsx,tsx}'],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        ink: {
          950: '#07080c',
          900: '#0d0f15',
          800: '#14161d',
          700: '#1b1e27',
        },
        accent: {
          DEFAULT: 'var(--accent)',
          hi: 'var(--accent-hi)',
          text: 'var(--accent-text)',
        },
      },
      borderRadius: {
        '2.5xl': '1.25rem',
      },
    },
  },
  plugins: [],
} satisfies Config;
