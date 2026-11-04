import { cva, type VariantProps } from 'class-variance-authority';

export { default as Button } from './Button.vue';

/** Desktop-sized buttons: 28px default (macOS/Windows control height), 32px for primary calls. */
export const buttonVariants = cva(
  'inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-md font-medium transition-[background-color,color,box-shadow,opacity] duration-150 outline-none focus-visible:ring-2 focus-visible:ring-ring/40 disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0',
  {
    variants: {
      variant: {
        default: 'bg-primary text-primary-foreground shadow-xs hover:bg-primary-hover',
        outline: 'border border-input bg-card text-foreground shadow-xs hover:bg-accent',
        secondary: 'bg-muted text-foreground hover:bg-accent',
        ghost: 'text-foreground hover:bg-foreground/[0.06]',
        soft: 'bg-primary-soft text-primary-soft-foreground hover:brightness-95',
        amber: 'bg-amber-foreground text-amber-soft hover:opacity-90',
        destructive: 'text-destructive-foreground hover:bg-destructive-soft',
        link: 'text-primary underline-offset-4 hover:underline',
      },
      size: {
        'xs': 'h-7 rounded-[6px] px-2.5 text-[12.5px] [&_svg]:size-3.5',
        'sm': 'h-8 px-3 text-[13px] [&_svg]:size-4',
        'lg': 'h-10 rounded-full px-6 text-[14px] font-semibold [&_svg]:size-4',
        'icon': 'size-8 [&_svg]:size-4',
        'icon-xs': 'size-7 rounded-[6px] [&_svg]:size-4',
      },
    },
    defaultVariants: { variant: 'default', size: 'xs' },
  },
);

export type ButtonVariants = VariantProps<typeof buttonVariants>;
