-- A bare **outer** column reference at a level whose FROM is an `unnest(…)` — the accept
-- shape the exhaustive flip is most likely to break, and the one the plan-time binding
-- gate's provisional-level rule exists for.
--
-- The subquery's level is lowered before the enclosing query's scope exists, so it stays
-- *provisional* until `resolve_correlated_subplans` re-walks it against the level-chained
-- scope. Closing such a level at its own lowering is the recorded trap: an ordinary
-- two-level correlated reference becomes `42P01`, because at the inner query's own
-- lowering the outer relation is not there yet. Making the unnest arm columns-exhaustive
-- must not disturb that — it changes what a *complete* chain says about a name, never
-- when the chain is complete.
--
-- Two further readers of the same field ride this statement and **cannot** move with the
-- flip, which is worth stating because the shape invites the opposite reading:
-- `ColumnScope::bare_candidates` and `qualified` consult `level_columns_uncertain` only to
-- decide whether to keep walking **outward**, and `build_column_scope` stamps every
-- relation level `0` — only `nested_in` builds a chain, and its one production caller
-- chains a scope `push_level_relations` built, whose `Unnest` arm was exhaustive all
-- along. So both answer identically on either side of the flip. What this fixture guards
-- is the accept polarity of the readers that *do* move (`binding_qualified`,
-- `binding_bare`, `sole_relation_carrying`): a widening that went too far — binding `id`
-- to the unnest's own `q`, or refusing it outright — shows up here.
--
-- The values are what make the correlation observable rather than merely the acceptance:
-- `id = 1` matches the single array element and reads `pt`'s `v`, `id = 2` matches
-- nothing and reads SQL NULL. A fixture whose subquery matched every row would answer
-- identically to one that ignored the correlation.
-- @setup
CREATE TABLE pt (id INT, v TEXT);
INSERT INTO pt VALUES (1, 'a'), (2, 'b');

-- @test
SELECT id, (SELECT v FROM unnest(ARRAY[1]) AS w(q) WHERE q = id) AS c FROM pt ORDER BY id;
