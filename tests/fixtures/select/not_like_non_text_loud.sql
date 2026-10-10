-- The highest-severity shape of the un-gated LIKE operand: `NOT LIKE` negated the
-- silent `false`, so a non-text left operand matched **every** row. Postgres has no
-- `integer !~~ text` operator, so this is 42883.
-- @setup
CREATE TABLE nl (id INT);
INSERT INTO nl VALUES (1), (2), (3);
-- @test
-- @expect-error: no integer NOT LIKE operator
SELECT id FROM nl WHERE id NOT LIKE '9%';
