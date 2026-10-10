-- 18.6 records the `pg_attrdef` → sequence NORMAL edge from the `regclass` constant
-- inside the cooked expression, not from the shape of its top node, so a
-- `DEFAULT nextval('s') + 1` blocks `DROP SEQUENCE s` exactly as the bare spelling
-- does. A resolver reading only the closed `NextVal` arm sees no reference here and
-- lets the sequence drop out from under a default that names it.
-- @setup
CREATE SEQUENCE def_edge_seq;
CREATE TABLE def_edge_t (a INT DEFAULT nextval('def_edge_seq') + 1);
-- @expect-error: the expression default's sequence edge refuses the drop
-- @test
DROP SEQUENCE def_edge_seq;
