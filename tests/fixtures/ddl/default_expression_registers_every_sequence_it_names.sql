-- 18.6 records a `pg_attrdef` → sequence dependency **per sequence function** in the
-- cooked expression, so `DEFAULT nextval('a') + nextval('b')` names two. pglet's
-- registration used to read the closed `ColumnDefault::NextVal` arm, which sees one
-- sequence or none, so the expression spelling recorded neither and every write
-- answered `42P01`. Both draws advance, once per row: 1 + 1 on the first row and
-- 2 + 2 on the second.
-- @setup
CREATE SEQUENCE multi_seq_a;
CREATE SEQUENCE multi_seq_b;
CREATE TABLE multi_seq_t (k INT, v INT DEFAULT nextval('multi_seq_a') + nextval('multi_seq_b'));
INSERT INTO multi_seq_t (k) VALUES (1);
INSERT INTO multi_seq_t (k) VALUES (2);
-- @test
SELECT k, v FROM multi_seq_t ORDER BY k;
