import { Metadata } from 'next';
import { translateUI } from '@/i18n';


export const metadata: Metadata = {
  title: 'HuiTrace',
  get description() { return translateUI("AI-powered meeting assistant"); },
};
