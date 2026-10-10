-- The same `LOWER(email)` expression unique index allows two rows whose computed
-- keys differ: 'a@x' and 'b@x' lowercase to distinct keys, so both insert.
-- @setup
CREATE TABLE expr_lower_distinct (id INT, email TEXT);
CREATE UNIQUE INDEX ux_expr_lower_distinct ON expr_lower_distinct (LOWER(email));
INSERT INTO expr_lower_distinct VALUES (1, 'a@x'), (2, 'b@x');

-- @test
SELECT id, email FROM expr_lower_distinct ORDER BY id;
