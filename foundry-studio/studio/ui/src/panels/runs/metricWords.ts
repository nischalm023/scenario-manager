// A metric's name for a reader (spec 08 section 3.9): the simulator declares metrics by their code names
// (`htm_duty_traveling_ms`, `rack_fill_max_pct`) and, so far, labels them the same, so the page spells those out:
// the unit suffix goes, since the value carries its unit; initialisms stay upper case; a state's name stays as
// declared. A label that differs from the code name is the simulator's own and is kept. The family a metric belongs
// to, by its first word, groups All metrics so a hundred rows read as a dozen lists.

const UPPER = new Set(['htm', 'vtm', 'sku', 'skus', 'pct', 'id']);
const SUFFIXES = ['_ms', '_mm', '_ns', '_count', '_pct'];
const WORDS: Record<string, string> = { avg: 'average', stddev: 'spread', max: 'highest', min: 'lowest', pct: '%', num: 'number of' };

/** `htm_duty_traveling_ms` reads "HTM duty traveling"; `rack_fill_max_pct` reads "Rack fill highest"; a label of the
 * simulator's own is returned as given. */
export function metricName(metric: string, label: string): string {
  if (label !== '' && label !== metric) return label;
  let stem = metric;
  for (const suffix of SUFFIXES) {
    if (stem.endsWith(suffix) && stem.length > suffix.length) {
      stem = stem.slice(0, -suffix.length);
      break;
    }
  }
  const words = stem.split('_').filter((word) => word !== '').map((word, at) => {
    const lower = word.toLowerCase();
    if (UPPER.has(lower)) return word.toUpperCase();
    if (WORDS[lower] !== undefined) return WORDS[lower];
    if (/^[A-Z]/.test(word)) return word.replace(/([a-z])([A-Z])/g, '$1 $2').toLowerCase();
    return at === 0 ? lower.charAt(0).toUpperCase() + lower.slice(1) : lower;
  });
  return words.join(' ').replace(/^state /i, 'State: ').replace(/ state /, ' state: ');
}

/** The family a metric is grouped under, by its first word: "HTM" for `htm_*`, "Picking" for `pick_*` and `picks_*`,
 * the family named below for another word it names, else the word itself; a metric of one word is "Other". */
export function metricFamily(metric: string): string {
  const [first, second] = metric.split('_');
  const lower = (first ?? '').toLowerCase();
  if (UPPER.has(lower)) return lower.toUpperCase();
  const pairs: Record<string, string> = {
    pick: 'Picking', picks: 'Picking', putaway: 'Putaway', putaways: 'Putaway', rack: 'Racks', relay: 'Relay', order: 'Orders', orders: 'Orders',
    diving: 'Diving board', pathfinder: 'Pathfinder', pipeline: 'Pipeline', tote: 'Totes', inventory: 'Inventory', operator: 'Operators',
    wall: 'Run', backpressure: 'Flow control', deadlock: 'Flow control', starvation: 'Flow control', presentations: 'Presentations',
  };
  if (pairs[lower] !== undefined) return pairs[lower];
  return second === undefined ? 'Other' : lower.charAt(0).toUpperCase() + lower.slice(1);
}
