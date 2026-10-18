import React from "react";
import type { ToastInfo } from "../hooks/useEvoSocket";

interface ToastBannerProps {
  toast: ToastInfo | null;
}

export const ToastBanner: React.FC<ToastBannerProps> = ({ toast }) => {
  if (!toast) return <div className="toast-banner" />;

  return (
    <div className={`toast-banner show ${toast.isError ? "error" : "info"}`}>
      {toast.text}
    </div>
  );
};
