-- The assignment-position counterpart of `params/unary_plus_param_typing.sql`:
-- `SET col = +$1` also types the parameter float8 from the operator, not int4 from
-- the target column — `record_value_param`'s column-driven rule does not apply
-- through a `+`.
-- @setup
CREATE TABLE supp (id INT, n INT);
INSERT INTO supp VALUES (1, 10), (2, 20);
-- @param $1 float8 99
-- @test
UPDATE supp SET n = +$1 WHERE id = 1 RETURNING id, n;
