-- `DEFAULT CURRENT_TIME` resolves the current UTC time-of-day at INSERT. The value
-- is a moving clock, so compare only that the column is populated and in range —
-- deterministic `true` on both engines.
-- @setup
CREATE TABLE t (id INT, created_at TIME DEFAULT CURRENT_TIME);
INSERT INTO t (id) VALUES (1);

-- @test
SELECT id,
       created_at IS NOT NULL          AS is_set,
       created_at >= '00:00:00'::time  AS after_midnight,
       created_at <= '24:00:00'::time  AS before_end
FROM t;
