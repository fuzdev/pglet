-- Error order, third of three, and the one that is not a *binding* error on either side:
-- the target list is transformed before the sort clause, so an operand pair with no
-- operator is reported ahead of the key's missing relation (18.6: `42883 operator does not
-- exist: text = integer`; this was the key's `42P01`). The projection being ill-typed
-- rather than unbound is what makes it a second class rather than a second spelling — the
-- two errors come from different passes over the same clause.
-- @setup
CREATE TABLE sk_ept (a INT, s TEXT);
INSERT INTO sk_ept VALUES (1, 'x'), (2, 'y');

-- @expect-error: operator does not exist: text = integer (42883)
-- @test
SELECT s = 5 FROM sk_ept ORDER BY bogusrel.a;
