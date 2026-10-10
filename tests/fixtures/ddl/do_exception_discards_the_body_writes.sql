-- A `DO` block with an `EXCEPTION` clause is a **subtransaction**: catching the
-- error discards everything the body wrote, so neither of the two rows the body
-- inserted before the violation survives. Measured on 18.6.
--
-- pglet used to keep them, and this is the worse polarity of the two: the error
-- is swallowed by design, so nothing is loud — the caller sees a `DO` command tag
-- over a half-applied body and has no way to tell.
-- @setup
CREATE TABLE do_sub (id INT PRIMARY KEY);
DO $$ BEGIN
    INSERT INTO do_sub VALUES (1);
    INSERT INTO do_sub VALUES (2);
    INSERT INTO do_sub VALUES (1);
EXCEPTION WHEN unique_violation THEN NULL;
END $$;

-- @test
SELECT count(*) AS rows_after_caught_do FROM do_sub;
