-- `ORDER BY` is the third clause the same walk reaches, and the one that pins the
-- clause **order**: pglet resolves the sort keys after the projection, so a statement
-- whose only fault is here still raises the signature refusal rather than answering.
-- With an *array* argument 18.6 sorts (a set-returning `ORDER BY` key is legal there),
-- so this fixture's polarity is the argument's type and not the position.
-- @setup
CREATE TABLE unnest_na_ord (i INT);
-- @test
-- @expect-error: no unnest(integer) in ORDER BY
SELECT i FROM unnest_na_ord ORDER BY unnest(i);
