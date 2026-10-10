-- A DO block whose body error does NOT match the EXCEPTION condition propagates
-- (it is not swallowed): inserting into a missing table raises 42P01
-- (undefined_table), which a `duplicate_object` handler does not catch. pglet
-- fails loud, matching Postgres.
-- @expect-error: unmatched exception condition propagates
-- @test
DO $$ BEGIN
    INSERT INTO no_such_table VALUES (1);
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;
