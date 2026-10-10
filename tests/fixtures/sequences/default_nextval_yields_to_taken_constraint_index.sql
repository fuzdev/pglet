-- ...and a declared constraint name the relation namespace already holds, which is the
-- third gate in that block (`index_create`'s relation check).
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @setup
CREATE TABLE dnix_host (a INT);
CREATE TABLE dnix_taken (q INT PRIMARY KEY);
-- @test
CREATE TABLE dnix (b INT DEFAULT nextval('dnix_host'), a INT, CONSTRAINT dnix_taken_pkey PRIMARY KEY (a));
-- @expect-error: the taken constraint-index name outranks the kind gate (42P07)
