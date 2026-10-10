-- The `ON CONFLICT … DO UPDATE SET` spelling, which is the **silent** one of the four
-- sites. The action is only evaluated when a row actually conflicts, so the store-time
-- refusal that stands behind the other sites is not reached here at all: this INSERT
-- hits **no** conflict, inserts its row, and reported success where 18.6 refuses the
-- statement outright. The conflicting spelling of the same statement is loud on both
-- engines either way, which is why the no-conflict one is the discriminator.
--
-- The right-hand side is `EXCLUDED`-qualified on purpose: the action's namespace is the
-- target plus the `excluded` pseudo-relation, not the catalog, and only a reference into
-- `excluded` says so.
--
-- Tagged `@profile-lock` because its `@setup` builds an index (the `PRIMARY KEY` the
-- arbiter needs) and the completeness check forces a decision on every such
-- `@expect-error` fixture. The refusal is raised in the INSERT lowering, before index
-- selection runs at all, so it cannot differ by build profile — but running it on the
-- stripped profile measures that rather than arguing it.
-- @profile-lock
-- @setup
CREATE TABLE acoc (id INT PRIMARY KEY, b BOOL);
INSERT INTO acoc (id, b) VALUES (1, TRUE);

-- @expect-error: column "b" is of type boolean but expression is of type integer (42804)
-- @test
INSERT INTO acoc (id, b) VALUES (2, FALSE) ON CONFLICT (id) DO UPDATE SET b = EXCLUDED.id;
