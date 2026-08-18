(ns app.api
  (:require [app.impl :as impl]))

(defn run [value]
  (impl/parse value))
