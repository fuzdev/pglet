/**
 * Kysely spy capture script.
 *
 * Run this through the spy proxy to capture what queries Kysely generates.
 * NOT run in CI - this is a development tool.
 *
 * Usage:
 *   Terminal 1: deno task spy:start
 *   Terminal 2: deno run -A tests-web/orms/kysely/capture.ts
 *
 * Check tests/captures/ for captured queries.
 */

import { Kysely, PostgresDialect, sql } from "kysely";
import pg from "pg";
import type { Database } from "./types.ts";

const { Pool } = pg;

const db = new Kysely<Database>({
  dialect: new PostgresDialect({
    pool: new Pool({
      host: "127.0.0.1",
      port: 5434, // spy proxy port
      user: "postgres",
      database: "postgres",
    }),
  }),
});

console.log("=== Kysely Spy Capture ===\n");

try {
  // Schema setup
  console.log("1. Creating tables...");
  await sql`DROP TABLE IF EXISTS posts`.execute(db);
  await sql`DROP TABLE IF EXISTS users`.execute(db);
  await sql`
    CREATE TABLE users (
      id INT PRIMARY KEY,
      name TEXT NOT NULL,
      email TEXT NOT NULL,
      active BOOL DEFAULT true,
      created_at TIMESTAMP
    )
  `.execute(db);
  await sql`
    CREATE TABLE posts (
      id INT PRIMARY KEY,
      title TEXT NOT NULL,
      content TEXT,
      author_id INT NOT NULL,
      published BOOL DEFAULT false
    )
  `.execute(db);

  // INSERT operations
  console.log("2. Testing INSERT...");
  await db.insertInto("users").values({ id: 1, name: "Alice", email: "alice@example.com" }).execute();
  await db.insertInto("users").values([
    { id: 2, name: "Bob", email: "bob@example.com" },
    { id: 3, name: "Carol", email: "carol@example.com" },
  ]).execute();

  // SELECT operations
  console.log("3. Testing SELECT...");
  await db.selectFrom("users").selectAll().execute();
  await db.selectFrom("users").select(["name", "email"]).execute();
  await db.selectFrom("users").selectAll().where("id", "=", 1).execute();
  await db.selectFrom("users").selectAll().where("id", ">", 1).execute();
  await db.selectFrom("users").selectAll().where("id", "=", 1).where("name", "=", "Alice").execute();
  await db.selectFrom("users").selectAll().where((eb) => eb.or([eb("id", "=", 1), eb("id", "=", 3)])).execute();
  await db.selectFrom("users").selectAll().orderBy("name", "asc").execute();
  await db.selectFrom("users").selectAll().orderBy("id", "desc").execute();
  await db.selectFrom("users").selectAll().orderBy("id", "asc").limit(2).execute();
  await db.selectFrom("users").selectAll().orderBy("id", "asc").limit(2).offset(1).execute();

  // UPDATE operations
  console.log("4. Testing UPDATE...");
  await db.updateTable("users").set({ name: "Alicia" }).where("id", "=", 1).execute();
  await db.updateTable("users").set({ name: "Bob Jr", active: false }).where("id", "=", 2).execute();

  // DELETE operations
  console.log("5. Testing DELETE...");
  await db.deleteFrom("users").where("id", "=", 3).execute();
  await db.deleteFrom("users").execute();

  // Multi-table operations
  console.log("6. Testing multi-table...");
  await db.insertInto("users").values({ id: 1, name: "Alice", email: "a@ex.com" }).execute();
  await db.insertInto("posts").values([
    { id: 1, title: "First Post", content: "Hello", author_id: 1 },
    { id: 2, title: "Second Post", content: "World", author_id: 1 },
  ]).execute();
  await db.selectFrom("posts").selectAll().where("author_id", "=", 1).execute();

  // Cleanup
  console.log("7. Cleanup...");
  await sql`DROP TABLE posts`.execute(db);
  await sql`DROP TABLE users`.execute(db);

  console.log("\n=== Capture Complete ===");
  console.log("Check tests/captures/ for captured queries.");
} catch (e) {
  console.error("Error:", e);
  throw e;
} finally {
  await db.destroy();
}
