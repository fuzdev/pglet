-- Two enums over the **same two labels in opposite orders**: the order is the *type's*,
-- not a property of the labels, so `ea`'s minimum is `x` and `eb`'s is `y`. A fix that
-- read one global label order — or that keyed the order on anything but the column's own
-- type — cannot answer both, and `ea` is its own control (its declared order is also its
-- lexical one, so only `eb` moves).
-- @setup
CREATE TYPE ea AS ENUM ('x', 'y');
CREATE TYPE eb AS ENUM ('y', 'x');
CREATE TABLE ea_t (v ea NOT NULL);
CREATE TABLE eb_t (v eb NOT NULL);
INSERT INTO ea_t VALUES ('x'), ('y');
INSERT INTO eb_t VALUES ('x'), ('y');

-- @test
SELECT
  (SELECT min(v)::text FROM ea_t) AS a_lo,
  (SELECT max(v)::text FROM ea_t) AS a_hi,
  (SELECT min(v)::text FROM eb_t) AS b_lo,
  (SELECT max(v)::text FROM eb_t) AS b_hi;
