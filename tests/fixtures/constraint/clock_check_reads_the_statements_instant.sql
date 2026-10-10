-- A `CHECK` that compares a column with `now()` reads the instant the statement stamped
-- the column with, so a row written from `now()` satisfies `CHECK (at = now())` — through
-- a written value, a `DEFAULT`, and an `UPDATE`.
-- @setup
CREATE TABLE ccr (id INT, at TIMESTAMPTZ DEFAULT now() CHECK (at = now()), day DATE DEFAULT CURRENT_DATE CHECK (day = CURRENT_DATE));
INSERT INTO ccr (id, at) VALUES (1, now()), (2, CURRENT_TIMESTAMP), (3, transaction_timestamp());
INSERT INTO ccr (id) VALUES (4), (5);
UPDATE ccr SET at = now() WHERE id <= 2;
-- @test
SELECT count(*) AS accepted_rows, count(DISTINCT at) AS at_values FROM ccr;
