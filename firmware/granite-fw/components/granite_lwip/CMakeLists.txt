# lwIP helpers that need lwIP's own struct layouts (platform/net.rs).
idf_component_register(SRCS "granite_lwip.c" PRIV_REQUIRES lwip esp_netif)
