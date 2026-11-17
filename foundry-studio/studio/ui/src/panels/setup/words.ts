// How the Setup pages write a count: grouped by thousands, as the other panels write theirs.

export const grouped = (value: number): string => value.toLocaleString('en-US');
