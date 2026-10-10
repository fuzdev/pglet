-- pg_dump's preamble calls schema-qualified pg_catalog functions: set_config
-- (restores search_path) and setval (restores a sequence's value). pglet must
-- parse `pg_catalog.fn(...)` and execute both: set_config echoes its value arg,
-- setval echoes n (bigint).
-- @setup
CREATE SEQUENCE scfg_seq START WITH 1;
-- @test
SELECT pg_catalog.set_config('search_path', 'public', false) AS cfg,
       pg_catalog.setval('scfg_seq', 10, true) AS sv;
