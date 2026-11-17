/* The report's drawing half. Everything it knows comes from the JSON the
   generator embedded above it; it decides nothing about the world, only when to
   put a piece of it on screen.

   Two rules run through all of it. Nothing from a document is ever written as
   markup — every string arrives through textContent or a text node, so a name
   containing an element is a name and not an element. And no branch of the tree
   is built before somebody asks for it: a world of 21,000 entities laid out at
   once is a page that takes seconds to open and megabytes to hold, so a node is
   built when it is opened, siblings arrive a hundred at a time, and every
   shortened list says how many are left. */

(function () {
  'use strict';

  var data = JSON.parse(document.getElementById('world-data').textContent);
  var E = data.entities;
  var CHUNK = 100;
  var UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g;

  /* ---------------------------------------------------------------- lookups */

  /* An index past the end of the entity array names something the world no
     longer holds — a suppressed target's generator, an entity a record
     deleted — and those can be printed but never linked to. */
  function uuidAt(index) {
    return index < E.length ? E[index].i : data.absentIds[index - E.length];
  }

  function present(index) {
    return index < E.length;
  }

  var indexById = new Map();
  E.forEach(function (entity, index) {
    indexById.set(entity.i, index);
  });

  var declaredPos = new Map();
  data.declared.forEach(function (index, position) {
    declaredPos.set(index, position);
  });

  /* Which entities point at this one. It is built by turning the outbound
     references around rather than by scanning for UUID-shaped strings, so an
     arrow drawn backwards here is one the schema states. */
  var inbound = new Map();
  E.forEach(function (entity, index) {
    var row = entity.r || [];
    for (var i = 0; i < row.length; i += 2) {
      var target = row[i + 1];
      if (!inbound.has(target)) inbound.set(target, []);
      inbound.get(target).push([index, row[i]]);
    }
  });

  /* Arcs, by the entity each end of them touches: what leads out of a cell,
     what reaches it, and what generator or stitch compiled them. A stitch link
     is one arc that goes both ways (SPEC §5), so it is filed under both ends of
     both directions and a reader sees it from whichever cell they are standing
     on. */
  var arcsOut = new Map();
  var arcsIn = new Map();
  var arcsOwned = new Map();
  function push(map, key, value) {
    if (!map.has(key)) map.set(key, []);
    map.get(key).push(value);
  }
  for (var e = 0; e + 3 < data.edges.length; e += 4) {
    var arc = e / 4;
    push(arcsOwned, data.edges[e], arc);
    push(arcsOut, data.edges[e + 1], arc);
    push(arcsIn, data.edges[e + 2], arc);
    if (data.edges[e + 3]) {
      push(arcsOut, data.edges[e + 2], arc);
      push(arcsIn, data.edges[e + 1], arc);
    }
  }
  function arcEnds(arc) {
    return { from: data.edges[arc * 4 + 1], to: data.edges[arc * 4 + 2] };
  }

  /* Where each derived entity sits in the derivation tree: the generator it
     hangs under, then one child position per step of its key. It is what turns
     a link into a scroll — the branch has to be built before there is anything
     to scroll to. */
  var trailOf = new Map();
  var segmentOf = new Map();
  var generatorByEntity = new Map();
  data.generators.forEach(function (generator, position) {
    generator._at = position;
    if (generator.entity != null) generatorByEntity.set(generator.entity, generator);
    walk(generator.children, [position]);
  });
  function walk(nodes, trail) {
    for (var i = 0; i < nodes.length; i++) {
      var node = nodes[i];
      var here = trail.concat(i);
      if (node.e != null) {
        trailOf.set(node.e, here);
        segmentOf.set(node.e, node.s);
      }
      if (node.c) walk(node.c, here);
    }
  }

  /* The derivation address of a derived entity: the generator it was derived
     from, and the chain of key segments that reaches it. */
  function addressOf(index) {
    var trail = trailOf.get(index);
    if (!trail) return null;
    var generator = data.generators[trail[0]];
    var nodes = generator.children;
    var segments = [];
    for (var level = 1; level < trail.length; level++) {
      var node = nodes[trail[level]];
      segments.push(node.s);
      nodes = node.c || [];
    }
    return { generator: generator, key: segments.join('/') };
  }

  /* ------------------------------------------------------------- elements */

  function el(tag, className, text) {
    var node = document.createElement(tag);
    if (className) node.className = className;
    if (text != null) node.textContent = text;
    return node;
  }

  /* Enough of an identity to recognise it by, beside a name that already says
     what the thing is. A declared entity has only its UUID, so the first block
     of it is the handle people read; a derived entity's identity is its whole
     address, and what tells it from its siblings is the last step of the chain
     rather than the generator every one of them shares. */
  function shortId(id) {
    var cut = id.lastIndexOf('/');
    return cut < 0 ? id.slice(0, 8) : id.slice(cut + 1);
  }

  function labelOf(index) {
    if (!present(index)) return uuidAt(index);
    var entity = E[index];
    if (entity.n) return entity.n;
    var segment = segmentOf.get(index);
    return segment ? segment : shortId(entity.i);
  }

  function entityLink(index, text) {
    var uuid = uuidAt(index);
    if (!present(index)) return el('span', 'uuid', uuid);
    var link = el('a', null, text == null ? labelOf(index) : text);
    link.href = '#e-' + uuid;
    link.setAttribute('data-entity', uuid);
    return link;
  }

  function count(shown, total, noun) {
    return 'showing ' + shown.toLocaleString() + ' of ' + total.toLocaleString() + ' ' + noun;
  }

  /* A list that arrives in pieces. Nothing is ever dropped: what is not on
     screen is one button away, and the row says how much that is. */
  function chunked(items, render, noun) {
    var box = el('div', 'chunk');
    box._items = items;
    box._render = render;
    box._noun = noun;
    box._shown = 0;
    grow(box, CHUNK);
    return box;
  }

  function grow(box, more) {
    if (box._row) box.removeChild(box._row);
    box._row = null;
    var upTo = Math.min(box._items.length, box._shown + more);
    for (var i = box._shown; i < upTo; i++) box.appendChild(box._render(box._items[i], i));
    box._shown = upTo;
    if (upTo < box._items.length) {
      var row = el('div', 'more-row');
      row.appendChild(el('span', null, count(upTo, box._items.length, box._noun)));
      var next = Math.min(CHUNK, box._items.length - upTo);
      var another = el('button', null, 'show ' + next.toLocaleString() + ' more');
      another.type = 'button';
      another.addEventListener('click', function () {
        grow(box, CHUNK);
      });
      row.appendChild(another);
      var everything = el('button', null, 'show all ' + box._items.length.toLocaleString());
      everything.type = 'button';
      everything.addEventListener('click', function () {
        grow(box, box._items.length);
      });
      row.appendChild(everything);
      box.appendChild(row);
      box._row = row;
    }
  }

  function ensureShown(box, upTo) {
    if (box._shown < upTo) grow(box, upTo - box._shown);
  }

  /* A collapsible whose contents are built the first time it opens. */
  function collapsible(className, summary, build) {
    var details = el('details', className);
    details.appendChild(summary);
    details._build = build;
    details.addEventListener('toggle', function () {
      if (details.open) ensureBody(details);
    });
    return details;
  }

  function ensureBody(details) {
    if (details._built) return;
    details._built = true;
    var body = details._build();
    if (body) details.appendChild(body);
  }

  /* ---------------------------------------------------------------- nodes */

  function entitySummary(index, segment) {
    var entity = E[index];
    var summary = el('summary');
    if (segment) summary.appendChild(el('span', 'segment', segment));
    if (entity.n) summary.appendChild(el('span', 'ename', (segment ? ' ' : '') + entity.n));
    summary.appendChild(el('span', 'uuid-short', ' ' + shortId(entity.i)));
    var names = Object.keys(entity.c);
    summary.appendChild(el('span', 'tag', names.length ? names.join(' ') : 'no components'));
    return summary;
  }

  function entityNode(index, segment, className) {
    var details = collapsible(className, entitySummary(index, segment), function () {
      return entityBody(details, index);
    });
    details.id = 'e-' + E[index].i;
    return details;
  }

  function fact(list, term, value) {
    list.appendChild(el('dt', null, term));
    var dd = el('dd');
    if (typeof value === 'string') dd.textContent = value;
    else dd.appendChild(value);
    list.appendChild(dd);
  }

  function entityBody(details, index) {
    var entity = E[index];
    var body = el('div', 'body');
    var facts = el('dl', 'facts');
    fact(facts, 'id', entity.i);
    if (entity.d) {
      fact(facts, 'origin', 'declared — authored in a document');
    } else {
      var address = addressOf(index);
      if (address) {
        var line = el('span');
        line.appendChild(
          address.generator.entity != null
            ? entityLink(address.generator.entity, address.generator.name || shortId(address.generator.id))
            : el('span', 'uuid', address.generator.id)
        );
        line.appendChild(el('span', 'segment', ' / ' + address.key));
        fact(facts, 'derived from', line);
      }
    }
    if (entity.p != null && present(entity.p)) fact(facts, 'parent', entityLink(entity.p));
    body.appendChild(facts);

    references(body, index);
    arcs(body, index);
    body.appendChild(payload(index));

    var trail = trailOf.get(index);
    var generator = generatorByEntity.get(index);
    if (generator) body.appendChild(derivedGroup(details, generator));
    if (trail) {
      var node = nodeAt(trail);
      if (node && node.c && node.c.length) {
        details._children = chunked(
          node.c,
          function (child, position) {
            return treeNode(child, trail.concat(position));
          },
          'entities'
        );
        body.appendChild(details._children);
      }
    }
    return body;
  }

  function nodeAt(trail) {
    var nodes = data.generators[trail[0]].children;
    var node = null;
    for (var level = 1; level < trail.length; level++) {
      node = nodes[trail[level]];
      nodes = node.c || [];
    }
    return node;
  }

  /* One step of a derivation chain. A step can carry no entity of its own — a
     stamp's `instance:0` is a level of the chain whether or not anything stands
     at it — and then it is drawn as the chain link it is. */
  function treeNode(node, trail) {
    if (node.e != null) return entityNode(node.e, node.s, 'node');
    var summary = el('summary');
    summary.appendChild(el('span', 'segment', node.s));
    summary.appendChild(el('span', 'tag', 'chain step'));
    var details = collapsible('node', summary, function () {
      var body = el('div', 'body');
      details._children = chunked(
        node.c || [],
        function (child, position) {
          return treeNode(child, trail.concat(position));
        },
        'entities'
      );
      body.appendChild(details._children);
      return body;
    });
    return details;
  }

  function derivedGroup(host, generator) {
    var summary = el('summary');
    summary.appendChild(
      el(
        'span',
        null,
        'derived — ' +
          generator.count.toLocaleString() +
          ' ' +
          (generator.count === 1 ? 'entity' : 'entities') +
          (generator.components.length
            ? ' (' +
              generator.components
                .map(function (kind) {
                  return kind[1].toLocaleString() + ' ' + kind[0];
                })
                .join(', ') +
              ')'
            : '')
      )
    );
    var group = collapsible('group', summary, function () {
      var box = chunked(
        generator.children,
        function (child, position) {
          return treeNode(child, [generator._at, position]);
        },
        'entities'
      );
      group._children = box;
      return box;
    });
    host._derived = group;
    return group;
  }

  function references(body, index) {
    var row = E[index].r || [];
    if (row.length) {
      var out = el('ul', 'refs');
      for (var i = 0; i < row.length; i += 2) {
        var item = el('li');
        item.appendChild(el('span', 'field', data.referenceKinds[row[i]] + ' → '));
        item.appendChild(entityLink(row[i + 1]));
        out.appendChild(item);
      }
      body.appendChild(labelled('references', out));
    }
    var back = inbound.get(index);
    if (back && back.length) {
      var list = el('ul', 'refs');
      var shown = back.slice(0, 40);
      shown.forEach(function (entry) {
        var item = el('li');
        item.appendChild(entityLink(entry[0]));
        item.appendChild(el('span', 'field', ' ← ' + data.referenceKinds[entry[1]]));
        list.appendChild(item);
      });
      if (back.length > shown.length) {
        list.appendChild(el('li', 'more-row', count(shown.length, back.length, 'references in')));
      }
      body.appendChild(labelled('referenced by', list));
    }
  }

  function labelled(term, content) {
    var wrapper = el('div', 'labelled');
    wrapper.appendChild(el('div', 'field', term));
    wrapper.appendChild(content);
    return wrapper;
  }

  function arcs(body, index) {
    var owned = arcsOwned.get(index) || [];
    var out = arcsOut.get(index) || [];
    var into = arcsIn.get(index) || [];
    if (!owned.length && !out.length && !into.length) return;
    var facts = el('dl', 'facts');
    if (owned.length) fact(facts, 'arcs compiled here', owned.length.toLocaleString());
    if (out.length) fact(facts, 'leads to', endpoints(out, index, 'arcs out'));
    if (into.length) fact(facts, 'reached from', endpoints(into, index, 'arcs in'));
    body.appendChild(facts);
  }

  function endpoints(arcList, index, noun) {
    var box = el('span');
    var shown = arcList.slice(0, 12);
    shown.forEach(function (arc, position) {
      var ends = arcEnds(arc);
      var other = ends.from === index ? ends.to : ends.from;
      if (position) box.appendChild(document.createTextNode(', '));
      box.appendChild(entityLink(other));
    });
    if (arcList.length > shown.length) {
      box.appendChild(
        document.createTextNode(' … ' + count(shown.length, arcList.length, noun))
      );
    }
    return box;
  }

  /* The components as they were written, pretty-printed, with any UUID in them
     that names an entity of this world linked to it — the generator inside a
     cell address as much as a plain reference field.

     That is not a claim that the field is a reference: which fields are
     references is part of each component's schema and the list above this block
     is the schema's word on it. Here it means only that the same identity is
     somewhere else on this page. */
  function payload(index) {
    var entity = E[index];
    var names = Object.keys(entity.c);
    var summary = el('summary', null, 'components (' + names.length + ')');
    return collapsible('payload', summary, function () {
      var text = JSON.stringify(entity.c, null, 2);
      var pre = el('pre');
      var last = 0;
      var match;
      UUID.lastIndex = 0;
      while ((match = UUID.exec(text)) !== null) {
        var target = indexById.get(match[0]);
        if (target == null) continue;
        pre.appendChild(document.createTextNode(text.slice(last, match.index)));
        pre.appendChild(entityLink(target, match[0]));
        last = match.index + match[0].length;
      }
      pre.appendChild(document.createTextNode(text.slice(last)));
      return pre;
    });
  }

  /* ----------------------------------------------------------------- tree */

  var tree = document.getElementById('tree');
  var declaredBox = null;
  if (tree) {
    declaredBox = chunked(
      data.declared,
      function (index) {
        return entityNode(index, null, 'entity');
      },
      'declared entities'
    );
    tree.appendChild(declaredBox);
    var orphans = data.generators.filter(function (generator) {
      return generator.entity == null;
    });
    if (orphans.length) {
      var summary = el(
        'summary',
        null,
        'expansions whose generator was itself suppressed (' + orphans.length + ')'
      );
      tree.appendChild(
        collapsible('group', summary, function () {
          var box = el('div');
          orphans.forEach(function (generator) {
            var head = el('summary');
            head.appendChild(el('span', 'ename', generator.name || 'unnamed'));
            head.appendChild(el('span', 'uuid-short', ' ' + shortId(generator.id)));
            box.appendChild(
              collapsible('group', head, function () {
                return chunked(
                  generator.children,
                  function (child, position) {
                    return treeNode(child, [generator._at, position]);
                  },
                  'entities'
                );
              })
            );
          });
          return box;
        })
      );
    }
  }

  /* Build whatever branch an entity sits on, so there is something to scroll
     to. Chunks are grown only as far as the entity being reached. */
  function materialise(index) {
    var trail = trailOf.get(index);
    if (!trail) {
      var position = declaredPos.get(index);
      if (position != null && declaredBox) ensureShown(declaredBox, position + 1);
      return;
    }
    var generator = data.generators[trail[0]];
    var host;
    if (generator.entity != null && declaredBox) {
      var at = declaredPos.get(generator.entity);
      if (at == null) return;
      ensureShown(declaredBox, at + 1);
      var root = document.getElementById('e-' + E[generator.entity].i);
      if (!root) return;
      ensureBody(root);
      if (!root._derived) return;
      ensureBody(root._derived);
      host = root._derived._children;
    } else {
      return;
    }
    for (var level = 1; level < trail.length && host; level++) {
      ensureShown(host, trail[level] + 1);
      var child = host.children[trail[level]];
      if (!child) return;
      ensureBody(child);
      host = child._children;
    }
  }

  function reveal(index) {
    if (index == null || !present(index)) return null;
    var found = document.getElementById('e-' + E[index].i);
    if (!found) {
      materialise(index);
      found = document.getElementById('e-' + E[index].i);
    }
    if (!found) return null;
    for (var node = found.parentElement; node; node = node.parentElement) {
      if (node.tagName === 'DETAILS') {
        ensureBody(node);
        node.open = true;
      }
    }
    ensureBody(found);
    found.open = true;
    found.scrollIntoView({ block: 'center' });
    found.classList.remove('target');
    void found.offsetWidth;
    found.classList.add('target');
    return found;
  }

  document.addEventListener('click', function (event) {
    var link = event.target.closest ? event.target.closest('a[data-entity]') : null;
    if (!link) return;
    var index = indexById.get(link.getAttribute('data-entity'));
    if (index == null) return;
    event.preventDefault();
    if (reveal(index)) history.pushState(null, '', '#e-' + E[index].i);
  });

  window.addEventListener('popstate', function () {
    fromHash();
  });

  function fromHash() {
    var hash = window.location.hash;
    if (hash.indexOf('#e-') !== 0) return;
    reveal(indexById.get(hash.slice(3)));
  }

  var collapse = document.getElementById('collapse-all');
  if (collapse) {
    collapse.addEventListener('click', function () {
      var open = document.querySelectorAll('#tree details[open]');
      for (var i = 0; i < open.length; i++) open[i].open = false;
    });
  }

  /* --------------------------------------------------------- suppressions */

  var suppressions = document.getElementById('suppressions');
  if (suppressions && data.suppressions.total) {
    var authored = data.suppressions.authored;
    if (authored.length) {
      suppressions.appendChild(
        collapsible(
          'group',
          el('summary', null, 'written as suppression records (' + authored.length.toLocaleString() + ')'),
          function () {
            return chunked(authored, suppressedRow, 'suppressions');
          }
        )
      );
    }
    data.suppressions.byExclusion.forEach(function (group) {
      var summary = el('summary');
      summary.appendChild(el('span', null, 'carved out of '));
      summary.appendChild(entityLink(group.x));
      summary.appendChild(el('span', null, ' — ' + group.count.toLocaleString() + ' cells'));
      suppressions.appendChild(
        collapsible('group', summary, function () {
          return chunked(group.items, suppressedRow, 'cells');
        })
      );
    });
  }

  function suppressedRow(item) {
    var row = el('div', 'refs');
    row.appendChild(entityLink(item.a));
    if (item.k) row.appendChild(el('span', 'segment', ' / ' + item.k));
    else row.appendChild(el('span', 'field', ' (a declared entity)'));
    return row;
  }

  /* ----------------------------------------------------------------- find */

  var find = document.getElementById('find');
  var results = document.getElementById('find-results');
  if (find && results) {
    find.addEventListener('input', function () {
      var query = find.value.trim().toLowerCase();
      results.textContent = '';
      if (query.length < 2) {
        results.classList.remove('open');
        return;
      }
      var hits = [];
      var total = 0;
      for (var i = 0; i < E.length; i++) {
        var entity = E[i];
        var segment = segmentOf.get(i);
        var hit =
          entity.i.indexOf(query) >= 0 ||
          (entity.n && entity.n.toLowerCase().indexOf(query) >= 0) ||
          (segment && segment.toLowerCase().indexOf(query) >= 0);
        if (!hit) continue;
        total++;
        if (hits.length < 50) hits.push(i);
      }
      if (!total) {
        results.appendChild(el('span', 'hit-note', 'nothing matches'));
        results.classList.add('open');
        return;
      }
      hits.forEach(function (index) {
        var line = el('a', 'hit');
        line.href = '#e-' + E[index].i;
        line.setAttribute('data-entity', E[index].i);
        line.appendChild(el('span', 'ename', labelOf(index)));
        line.appendChild(el('span', 'uuid-short', ' ' + shortId(E[index].i)));
        var names = Object.keys(E[index].c);
        if (names.length) line.appendChild(el('span', 'tag', names.join(' ')));
        results.appendChild(line);
      });
      results.appendChild(
        el(
          'span',
          'hit-note',
          total > hits.length ? count(hits.length, total, 'matches') : total.toLocaleString() + ' matches'
        )
      );
      results.classList.add('open');
    });
    document.addEventListener('click', function (event) {
      if (!results.contains(event.target) && event.target !== find) results.classList.remove('open');
    });
  }

  fromHash();
})();
