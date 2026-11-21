import { createTheme } from '@ox-content/vite-plugin';
import { Landing } from './layouts/Landing.tsx';

export const theme = createTheme({ layouts: { default: Landing } });
