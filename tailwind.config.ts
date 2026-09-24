import type { Config } from 'tailwindcss';

export default {
  content: ['./index.html', './src/**/*.{js,ts,jsx,tsx}'],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        ink: {
          950: '#0b0d14',
          900: '#0f121b',
          800: '#151927',
          700: '#1c2133',
        },
        accent: {
          DEFAULT: '#8b7cf6',
          2: '#38bdf8',
        },
      },
      borderRadius: {
        '2.5xl': '1.25rem',
      },
    },
  },
  plugins: [],
} satisfies Config;
