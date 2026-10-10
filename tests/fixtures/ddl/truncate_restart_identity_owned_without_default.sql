-- The restart set comes from the ownership edge, so a sequence owned by a column that
-- carries **no** default is still restarted — a shape the column-default reading
-- misses entirely.
-- @setup
CREATE TABLE triow_t (a INT);
CREATE SEQUENCE triow_seq START WITH 50;
ALTER SEQUENCE triow_seq OWNED BY triow_t.a;
INSERT INTO triow_t (a) VALUES (nextval('triow_seq')), (nextval('triow_seq'));
TRUNCATE triow_t RESTART IDENTITY;
-- @test
SELECT nextval('triow_seq');
