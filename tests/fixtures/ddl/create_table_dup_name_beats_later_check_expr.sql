-- Error order between the *constraints of one list*. Postgres cooks and names each
-- constraint in turn (`AddRelationNewConstraints`), so constraint 2's duplicate **name**
-- is reported before constraint 3's ill-typed **expression**. Validating every
-- expression in one pass and then every name in another cannot produce this: it answers
-- 42883 here, and the reverse arrangement (below) answers 42883 too, so no pass ordering
-- yields both.
-- @expect-error: constraint n for relation ctdn already exists
-- @test
CREATE TABLE ctdn (a INT, CONSTRAINT n CHECK (a > 0), CONSTRAINT n CHECK (a > 1),
                   CONSTRAINT m CHECK (a LIKE 'x'));
