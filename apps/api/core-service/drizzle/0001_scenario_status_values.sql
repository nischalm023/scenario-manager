ALTER TYPE "public"."scenario_status" RENAME VALUE 'active' TO 'running';--> statement-breakpoint
ALTER TYPE "public"."scenario_status" RENAME VALUE 'completed' TO 'finished';--> statement-breakpoint
ALTER TYPE "public"."scenario_status" ADD VALUE 'paused' BEFORE 'finished';
