-- The numeric release a client feature-gates on, compared against the oracle
-- byte-for-byte. `SHOW server_version` itself cannot be a fixture — 18.6 answers it
-- with a packaging suffix (`18.6 (Debian 18.6-1.pgdg13+2)`) that varies per build
-- and that no client parses — so this is the half of the pair a differential run can
-- hold, and the unit tests hold the other half plus the startup `ParameterStatus`
-- string no fixture can see.
-- @test
SHOW server_version_num;
