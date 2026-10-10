-- The handler runs **after** the subtransaction has rolled back and outside it,
-- so what the handler writes survives while what the body wrote does not: `9`
-- stands and `5` is gone. Measured on 18.6.
--
-- This is the fixture that pins the *order* of the three storage operations. An
-- implementation that ran the handler first and rolled back afterwards would
-- lose the handler's row and still pass every other fixture here.
-- @profile-lock
-- @setup
CREATE TABLE do_handler (id INT PRIMARY KEY);
DO $$ BEGIN
    INSERT INTO do_handler VALUES (5);
    INSERT INTO do_handler VALUES (5);
EXCEPTION WHEN unique_violation THEN INSERT INTO do_handler VALUES (9);
END $$;

-- @test
SELECT id FROM do_handler ORDER BY id;
