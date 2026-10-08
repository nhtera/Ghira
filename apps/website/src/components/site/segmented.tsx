// SPDX-License-Identifier: Apache-2.0

// A small group of toggle buttons (aria-pressed), as the prototype's
// "Meeting language" and "Notes view" switches.

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  /** Language of the label, when it is not the page's (e.g. "vi"). */
  lang?: string;
}

export function Segmented<T extends string>({
  label,
  options,
  value,
  onChange,
  className,
}: {
  label: string;
  options: readonly SegmentedOption<T>[];
  value: T;
  onChange: (value: T) => void;
  className?: string;
}) {
  return (
    <div className={className ? `lang-switch ${className}` : "lang-switch"} role="group" aria-label={label}>
      {options.map((o) => (
        <button key={o.value} type="button" lang={o.lang} aria-pressed={o.value === value} onClick={() => o.value !== value && onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}
