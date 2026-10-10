-- A `nextval` inside an expression default advances **per inserted row**, exactly as
-- the bare `nextval` spelling does. Reading only the top node of the stored
-- expression would evaluate it once and copy one value into every row.
-- @setup
CREATE SEQUENCE def_seq_expr;
CREATE TABLE def_seq_t (k INT, a INT DEFAULT nextval('def_seq_expr') + 100);
INSERT INTO def_seq_t (k) VALUES (1);
INSERT INTO def_seq_t (k) VALUES (2);
INSERT INTO def_seq_t (k) VALUES (3);
-- @test
SELECT k, a FROM def_seq_t ORDER BY k;
