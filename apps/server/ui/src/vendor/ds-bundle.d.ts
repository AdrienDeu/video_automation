import type * as React from 'react';

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
  size?: 'md' | 'sm';
}
export declare function Button(props: ButtonProps): React.ReactElement;

export interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  label?: string;
  helperText?: string;
  error?: boolean | string;
}
export declare function Input(props: InputProps): React.ReactElement;

export interface BadgeProps {
  tone?: 'neutral' | 'accent' | 'success' | 'warning' | 'danger';
  children?: React.ReactNode;
}
export declare function Badge(props: BadgeProps): React.ReactElement;

export interface CardProps extends React.HTMLAttributes<HTMLDivElement> {
  padded?: boolean;
}
export declare function Card(props: CardProps): React.ReactElement;

export interface AlertProps {
  tone?: 'info' | 'success' | 'warning' | 'danger';
  title?: string;
  children?: React.ReactNode;
}
export declare function Alert(props: AlertProps): React.ReactElement;

export interface ProgressBarProps {
  value?: number;
  indeterminate?: boolean;
  label?: string;
}
export declare function ProgressBar(props: ProgressBarProps): React.ReactElement;

declare global {
  interface Window {
    VideoAutomation: {
      Button: typeof Button;
      Input: typeof Input;
      Badge: typeof Badge;
      Card: typeof Card;
      Alert: typeof Alert;
      ProgressBar: typeof ProgressBar;
    };
  }
}
