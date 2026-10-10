-- A quoted mixed-case spelling of `search_path` reaches the one-schema refusal
-- (`set_search_path_unknown_schema.sql`): 18.6 matches the name case-insensitively whatever
-- its quoting. Read verbatim it answered `SET`, and a harness that believed it had moved to its
-- own schema read and wrote `public`. Documentary: 18.6 accepts, so any pglet error passes.
-- @test
-- @expect-error: pglet has only the public schema, whatever the name's spelling
SET "Search_Path" = nosuch;
