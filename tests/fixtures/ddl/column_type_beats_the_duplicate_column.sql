-- The other side of each gate's position. The `tableElts` walk resolves every column's
-- type in parse analysis, ahead of `DefineRelation` entirely, so a refused type modifier
-- anywhere in the list outranks a repeat later in it — measured on 18.6, and the
-- `22023`/`42701` split is what makes this fixture discriminate without a message lock.
-- @expect-error: the refused char length outranks the duplicate column
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE ctbdc (a CHAR(0), b INT, b INT);
