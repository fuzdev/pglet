-- The other polarity, and without it every fixture above is satisfied by a block
-- that discards its body unconditionally: a body that **raises nothing** keeps
-- everything it wrote, `EXCEPTION` clause or not.
-- @profile-lock
-- @setup
CREATE TABLE do_ok (id INT PRIMARY KEY);
DO $$ BEGIN
    INSERT INTO do_ok VALUES (1);
    INSERT INTO do_ok VALUES (2);
EXCEPTION WHEN unique_violation THEN NULL;
END $$;

-- @test
SELECT id FROM do_ok ORDER BY id;
