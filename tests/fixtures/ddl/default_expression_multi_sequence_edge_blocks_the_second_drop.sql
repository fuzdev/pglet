-- The **second** sequence of a two-sequence expression default carries its own
-- `pg_attrdef` edge, so it blocks its `DROP` exactly as the first does. A walk that
-- stopped at the first `nextval` — or a registration that recorded only one — leaves
-- this one droppable out from under a default that names it.
-- @setup
CREATE SEQUENCE mdrop_a;
CREATE SEQUENCE mdrop_b;
CREATE TABLE mdrop_t (v INT DEFAULT nextval('mdrop_a') + nextval('mdrop_b'));
-- @expect-error: the second sequence's edge refuses the drop too
-- @test
DROP SEQUENCE mdrop_b;
