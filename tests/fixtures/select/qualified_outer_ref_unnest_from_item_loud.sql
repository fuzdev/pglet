-- A FROM-clause `unnest(…)` names a relation, and its scope must carry that name.
--
-- `single_table_name` had no `Plan::Unnest` arm, so this level's scope reported *no
-- relation identity* — which both consumers read as "a join": `SimpleColumns` drops the
-- qualifier, and the correlated outer frame offers its synthesized `"relation.column"`
-- tier. `v`'s column is quoted into existence as `"a.b"`, byte-identical to that
-- addressing, so the correlated `a.b` answered rows where 18.6 raises `42P01`.
--
-- The read counterpart of `update/correlated_outer_ref_unaliased_target_loud.sql`: the
-- same missing identity, at the other of the two sites that can lack one, so a fix
-- confined to either leaves the other green.
--
-- Controls measured against 18.6 and unmoved by the arm — the accept polarity lives in
-- `select/unnest_alias_qualified_references_resolve.sql`: the same reference spelled
-- `v.b` over a plain `b` column answers on both engines. The `zzz.b` spelling used to be
-- the *reject* control this arm did not reach, since a bare name the frame carries was
-- reached by the qualifier-blind fallback beneath this tier — which is what made the
-- quoted spelling the discriminating one, having no bare column to fall through to. That
-- fallback is gone (a qualifier no frame owns now leaves the chain), so `zzz.b` is
-- `42P01` on both engines too and this fixture and that one no longer differ in outcome —
-- only in which rule refuses them, which is why both are kept.
-- @setup
CREATE TABLE u2 (yv TEXT);
INSERT INTO u2 VALUES ('hit');

-- @expect-error: missing FROM-clause entry for table "a" (42P01)
-- @test
SELECT (SELECT count(*) FROM u2 WHERE u2.yv = a.b) AS n
  FROM unnest(ARRAY['hit', 'miss']) AS v("a.b");
