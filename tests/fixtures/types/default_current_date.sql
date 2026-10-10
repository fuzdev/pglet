-- `DEFAULT CURRENT_DATE` materializes the current UTC date for an omitted column
-- on INSERT (parallels `DEFAULT now()`). The stored value is non-deterministic, so
-- compare it against `CURRENT_DATE` rather than a fixed date.
-- @setup
CREATE TABLE t (id INT, created DATE DEFAULT CURRENT_DATE);
INSERT INTO t (id) VALUES (1);

-- @test
SELECT id, created = CURRENT_DATE AS is_today FROM t;
