-- An enum's conversion to text is stable on 18.6, so a comparison over it is no constant
-- and the `OR` keeps its right operand's aggregate.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE em (a INT, b INT NOT NULL);
INSERT INTO em VALUES (1, 10), (2, 0);
-- @expect-error: division by zero
-- @test
SELECT 'happy'::mood::text = 'happy' OR sum(1/b) > 0 FROM em;
