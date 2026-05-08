package main

import (
	"fmt"
	"log"
	"net/http"
	"os"

	"github.com/huichen/cobalt/internal/webui"
)

func main() {
	port := os.Getenv("PORT")
	if port == "" {
		port = "8080"
	}

	srv, err := webui.NewServer(webui.ServerOptions{
		APIKey:  os.Getenv("LLM_API_KEY"),
		BaseURL: os.Getenv("LLM_BASE_URL"),
		Model:   os.Getenv("LLM_MODEL"),
	})
	if err != nil {
		log.Fatalf("Failed to create server: %v", err)
	}

	addr := ":" + port
	log.Printf("pi-web starting on http://localhost%s", addr)
	if err := http.ListenAndServe(addr, srv); err != nil {
		log.Fatalf("Server error: %v", err)
	}
	fmt.Println("Server stopped")
}
