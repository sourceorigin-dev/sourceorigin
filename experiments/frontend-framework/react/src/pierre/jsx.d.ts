import 'react';

// Pierre declares this element for JSX in a file its package does not export,
// so the declaration is repeated here.
declare module 'react' {
  namespace JSX {
    interface IntrinsicElements {
      'diffs-container': React.DetailedHTMLProps<React.HTMLAttributes<HTMLElement>, HTMLElement>;
    }
  }
}
