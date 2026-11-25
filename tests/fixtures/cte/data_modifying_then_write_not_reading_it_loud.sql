-- A data-modifying `WITH` the body does not read runs after the body on 18.6, so it is
-- refused (`0A000`; 18.6 answers).
-- @setup
CREATE TABLE dmn (id INT);
CREATE TABLE dmn2 (id INT);
-- @expect-error: a writing body that does not read the data-modifying WITH
-- @test
WITH c AS (INSERT INTO dmn VALUES (1) RETURNING id) INSERT INTO dmn2 VALUES (2);
