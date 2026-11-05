/*
 * lwIP helpers for granite-fw (platform/net.rs).
 *
 * granite_autoip_yield: stop AutoIP once DHCP (or a static address) holds
 * the netif. lwIP 2.2 as shipped with ESP-IDF v5.5 does neither half of
 * this by itself: dhcp_bind() no longer calls autoip_stop() in
 * LWIP_DHCP_AUTOIP_COOP mode, and autoip_stop() only sets the state to OFF
 * and leaves AutoIP's ACD probe running, whose ACD_IP_OK callback then
 * calls autoip_bind() and overwrites the lease with 169.254/16 (seen on
 * rev C, 2026-11-04). So: take the ACD off the list first, then stop.
 *
 * Runs on the tcpip thread: call it through esp_netif_tcpip_exec().
 */
#include "esp_err.h"
#include "esp_log.h"
#include "lwip/acd.h"
#include "lwip/autoip.h"
#include "lwip/netif.h"
#include "lwip/prot/autoip.h"

esp_err_t granite_autoip_yield(void *ctx)
{
    struct netif *netif = (struct netif *)ctx;
    if (netif == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    struct autoip *autoip = netif_autoip_data(netif);
    if (autoip == NULL || autoip->state == AUTOIP_STATE_OFF) {
        return ESP_OK;
    }
    acd_remove(netif, &autoip->acd);
    autoip_stop(netif);
    ESP_LOGI("granite_lwip", "autoip stopped: the netif has a routable address");
    return ESP_OK;
}
